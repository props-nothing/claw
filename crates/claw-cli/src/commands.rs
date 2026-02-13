use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::error;

use claw_config::ConfigLoader;
use claw_runtime::AgentRuntime;

/// 🦞 Claw — Universal autonomous AI agent runtime
#[derive(Parser)]
#[command(name = "claw", version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Path to claw.toml config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Log level override (e.g. debug, info, warn, error)
    #[arg(short, long, global = true)]
    log_level: Option<String>,

    /// Enable verbose output (debug logging)
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress all log output (errors only)
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the agent runtime (gateway + channels + API server)
    Start {
        /// Don't start the API server
        #[arg(long)]
        no_server: bool,
    },
    /// Interactive chat in the terminal
    Chat {
        /// Session ID to resume (creates new if omitted)
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Show runtime status
    Status,
    /// Show version and build info
    Version,
    /// Show current configuration
    Config {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Show recent audit log entries
    Logs {
        /// Number of entries to show (default 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,

        /// Filter by event type (e.g. tool_execution, injection, budget)
        #[arg(short = 't', long)]
        event_type: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set a config value in claw.toml (dot-notation key)
    Set {
        /// Config key in dot notation (e.g. agent.model, autonomy.level)
        key: String,
        /// Value to set
        value: String,
    },
    /// Audit configuration for security issues
    Doctor,
    /// Initialize a new claw.toml in the current or home directory
    Init {
        /// Create in current directory instead of ~/.claw/
        #[arg(long)]
        local: bool,
    },
    /// Interactive setup wizard — configure your Claw agent step by step
    Setup {
        /// Create in current directory instead of ~/.claw/
        #[arg(long)]
        local: bool,
        /// Reset existing config before running the wizard
        #[arg(long)]
        reset: bool,
        /// Skip to a specific section: model, channels, autonomy, services
        #[arg(long)]
        section: Option<String>,
    },
    /// Generate shell completions for bash, zsh, or fish
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Manage skills (reusable multi-step workflows)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Run a standalone Skills Hub server for remote agents
    Hub {
        #[command(subcommand)]
        action: HubAction,
    },
    /// Mesh networking — manage peers and multi-agent coordination
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
    /// Manage channels — login, logout, status, approve pairing requests
    Channels {
        #[command(subcommand)]
        action: ChannelAction,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,
    /// Install a plugin from ClawHub
    Install { name: String, version: Option<String> },
    /// Uninstall a plugin
    Uninstall { name: String },
    /// Search ClawHub for plugins
    Search { query: String },
    /// Show detailed info about an installed plugin
    Info { name: String },
    /// Scaffold a new plugin project
    Create { name: String },
}

#[derive(Subcommand)]
enum SkillAction {
    /// List available skills
    List,
    /// Show details of a skill
    Show { name: String },
    /// Run a skill interactively
    Run {
        name: String,
        /// Parameters as key=value pairs
        #[arg(short, long, value_parser = parse_key_val)]
        param: Vec<(String, String)>,
    },
    /// Create a new skill definition
    Create { name: String },
    /// Delete a skill
    Delete { name: String },
    /// Publish a local skill to the Skills Hub
    Push { name: String },
    /// Pull a skill from the Skills Hub and install it locally
    Pull { name: String },
    /// Search the Skills Hub for skills
    Search {
        /// Search query
        query: String,
        /// Filter by tag
        #[arg(short, long)]
        tag: Option<String>,
    },
}

#[derive(Subcommand)]
enum HubAction {
    /// Start a standalone Skills Hub server that remote agents connect to
    Serve {
        /// Address to listen on (default: 0.0.0.0:3800)
        #[arg(short, long, default_value = "0.0.0.0:3800")]
        listen: String,
        /// Path to the hub database file (default: ~/.claw/hub.db)
        #[arg(short, long)]
        db: Option<String>,
    },
}

#[derive(Subcommand)]
enum MeshAction {
    /// Show mesh networking status (peer ID, connections, capabilities)
    Status,
    /// List all known peers in the mesh
    Peers,
    /// Send a direct message to a peer
    Send {
        /// Target peer ID
        peer_id: String,
        /// Message text
        message: String,
    },
}

#[derive(Subcommand)]
enum ChannelAction {
    /// Show status of all configured channels
    Status,
    /// Login / link a channel (QR code for WhatsApp, token for Telegram, etc.)
    Login {
        /// Channel type to login: whatsapp, telegram, discord, signal, slack
        channel: String,
        /// Account ID for multi-account setups
        #[arg(long)]
        account: Option<String>,
        /// Force re-login even if already linked
        #[arg(long)]
        force: bool,
    },
    /// Logout / unlink a channel
    Logout {
        /// Channel type: whatsapp, telegram, discord, signal, slack
        channel: String,
        /// Account ID for multi-account setups
        #[arg(long)]
        account: Option<String>,
    },
    /// List pending pairing requests (for channels using DM pairing)
    Pairing {
        /// Channel type: whatsapp, telegram, signal
        channel: String,
    },
    /// Approve a DM pairing request
    Approve {
        /// Channel type: whatsapp, telegram, signal
        channel: String,
        /// The pairing code to approve
        code: String,
    },
    /// Deny a DM pairing request
    Deny {
        /// Channel type: whatsapp, telegram, signal
        channel: String,
        /// The pairing code to deny
        code: String,
    },
}

/// Parse "key=value" CLI arguments.
fn parse_key_val(s: &str) -> std::result::Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

impl Cli {
    pub async fn run(self) -> claw_core::Result<()> {
        // Load config first so we can use it for log format
        let config_loader = ConfigLoader::load(self.config.as_deref())?;
        let config = config_loader.get();

        // Resolve log level: --verbose > --quiet > --log-level > config default
        let log_level = if self.verbose {
            "debug"
        } else if self.quiet {
            "error"
        } else {
            self.log_level.as_deref().unwrap_or("info")
        };

        // Initialize tracing with appropriate format
        if config.logging.format == "json" {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| {
                            tracing_subscriber::EnvFilter::new(log_level)
                        }),
                )
                .json()
                .with_target(true)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| {
                            tracing_subscriber::EnvFilter::new(log_level)
                        }),
                )
                .with_target(false)
                .init();
        }

        match self.command {
            Commands::Start { no_server } => {
                Self::cmd_start(config, no_server, config_loader).await
            }
            Commands::Chat { session } => {
                Self::cmd_chat(config, session).await
            }
            Commands::Status => {
                Self::cmd_status(config).await
            }
            Commands::Version => {
                Self::cmd_version()
            }
            Commands::Config { json } => {
                Self::cmd_config(config, json)
            }
            Commands::Plugin { action } => {
                Self::cmd_plugin(config, action).await
            }
            Commands::Logs { limit, event_type, json } => {
                Self::cmd_logs(config, limit, event_type, json).await
            }
            Commands::Set { key, value } => {
                Self::cmd_config_set(Some(config_loader.path().to_path_buf()), key, value)
            }
            Commands::Doctor => {
                Self::cmd_doctor(config)
            }
            Commands::Init { local } => {
                Self::cmd_init(local)
            }
            Commands::Setup { local, reset, section } => {
                Self::cmd_setup(local, reset, section)
            }
            Commands::Completions { shell } => {
                Self::cmd_completions(shell)
            }
            Commands::Skill { action } => {
                Self::cmd_skill(config, action, config_loader.path()).await
            }
            Commands::Hub { action } => {
                Self::cmd_hub(action).await
            }
            Commands::Mesh { action } => {
                Self::cmd_mesh(config, action).await
            }
            Commands::Channels { action } => {
                Self::cmd_channels(config, action).await
            }
        }
    }

    async fn cmd_start(config: claw_config::ClawConfig, no_server: bool, config_loader: ConfigLoader) -> claw_core::Result<()> {
        println!("🦞 Claw v{}", env!("CARGO_PKG_VERSION"));
        println!("   Model: {}", config.agent.model);
        println!("   Autonomy: L{}", config.autonomy.level);
        println!();

        // Start config hot-reload watcher (kept alive for duration of runtime)
        let _watcher = match config_loader.watch() {
            Ok(w) => {
                println!("   Config hot-reload: enabled");
                Some(w)
            }
            Err(e) => {
                tracing::warn!(error = %e, "config hot-reload disabled");
                None
            }
        };

        let mut runtime = AgentRuntime::new(config.clone())?;

        // Register LLM providers — config file keys take priority, env vars are fallback
        let mut providers_registered = 0u32;
        if let Some(ref key) = config.services.anthropic_api_key {
            let provider = claw_llm::anthropic::AnthropicProvider::new(key.clone());
            runtime.add_provider(Arc::new(provider));
            providers_registered += 1;
        }
        if let Some(ref key) = config.services.openai_api_key {
            let provider = claw_llm::openai::OpenAiProvider::new(key.clone());
            runtime.add_provider(Arc::new(provider));
            providers_registered += 1;
        }

        if providers_registered == 0 {
            let model = &config.agent.model;
            let is_local = model.starts_with("ollama/") || model.starts_with("local/");
            if !is_local {
                eprintln!("⚠️  No LLM API keys found. The agent won't be able to think.");
                eprintln!();
                if model.starts_with("anthropic/") {
                    eprintln!("   Your model is '{}'. Set your key:", model);
                    eprintln!("   In claw.toml:  [services]");
                    eprintln!("                  anthropic_api_key = \"sk-ant-...\"");
                    eprintln!("   Or env var:    export ANTHROPIC_API_KEY=sk-ant-...");
                } else if model.starts_with("openai/") {
                    eprintln!("   Your model is '{}'. Set your key:", model);
                    eprintln!("   In claw.toml:  [services]");
                    eprintln!("                  openai_api_key = \"sk-...\"");
                    eprintln!("   Or env var:    export OPENAI_API_KEY=sk-...");
                } else {
                    eprintln!("   Add your API keys to [services] in claw.toml:");
                    eprintln!("   anthropic_api_key = \"sk-ant-...\"");
                    eprintln!("   openai_api_key = \"sk-...\"");
                }
                eprintln!();
            }
        }

        // Register configured channels
        for (id, channel_config) in &config.channels {
            match channel_config.channel_type.as_str() {
                "telegram" => {
                    if let Some(token) = channel_config.settings.get("token").and_then(|v| v.as_str()) {
                        let channel = claw_channels::telegram::TelegramChannel::new(
                            id.clone(),
                            token.to_string(),
                        );
                        runtime.add_channel(Box::new(channel));
                    }
                }
                "webchat" => {
                    let channel = claw_channels::webchat::WebChatChannel::new(id.clone());
                    runtime.add_channel(Box::new(channel));
                }
                "whatsapp" | "wa" => {
                    let dm_policy_str = channel_config.settings.get("dm_policy")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pairing");
                    let dm_policy = match dm_policy_str {
                        "allowlist" => claw_channels::whatsapp::DmPolicy::Allowlist,
                        "open" => claw_channels::whatsapp::DmPolicy::Open,
                        "disabled" => claw_channels::whatsapp::DmPolicy::Disabled,
                        _ => claw_channels::whatsapp::DmPolicy::Pairing,
                    };
                    let allow_from: Vec<String> = channel_config.settings.get("allow_from")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    let channel = claw_channels::whatsapp::WhatsAppChannel::new(
                        id.clone(),
                        None,
                    )
                    .with_dm_policy(dm_policy)
                    .with_allow_from(allow_from);
                    runtime.add_channel(Box::new(channel));
                    println!("   📱 WhatsApp: enabled (dm_policy={})", dm_policy_str);
                    println!("      Link your phone: claw channels login whatsapp");
                }
                "discord" => {
                    if let Some(token) = channel_config.settings.get("token").and_then(|v| v.as_str()) {
                        let channel = claw_channels::discord::DiscordChannel::new(
                            id.clone(),
                            token.to_string(),
                        );
                        runtime.add_channel(Box::new(channel));
                    } else {
                        tracing::warn!("discord channel '{}' has no token configured", id);
                    }
                }
                "slack" => {
                    if let Some(token) = channel_config.settings.get("token").and_then(|v| v.as_str()) {
                        let app_token = channel_config.settings.get("app_token")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let channel = claw_channels::slack::SlackChannel::new(
                            id.clone(),
                            token.to_string(),
                            app_token,
                        );
                        runtime.add_channel(Box::new(channel));
                    } else {
                        tracing::warn!("slack channel '{}' has no token configured", id);
                    }
                }
                "signal" => {
                    let phone = channel_config.settings.get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !phone.is_empty() {
                        let channel = claw_channels::signal::SignalChannel::new(
                            id.clone(),
                            phone.to_string(),
                        );
                        runtime.add_channel(Box::new(channel));
                    } else {
                        tracing::warn!("signal channel '{}' has no phone number configured", id);
                    }
                }
                other => {
                    tracing::warn!(channel_type = other, "unsupported channel type");
                }
            }
        }

        if !no_server {
            // Start the API server in the background
            let server_config = config.server.clone();
            let hub_url = config.services.hub_url.clone();
            tokio::spawn(async move {
                if let Err(e) = claw_server::start_server(server_config, hub_url).await {
                    error!(error = %e, "API server failed");
                }
            });
        }

        // Run the agent runtime (blocks until shutdown)
        runtime.run().await
    }

    async fn cmd_chat(
        config: claw_config::ClawConfig,
        _session: Option<String>,
    ) -> claw_core::Result<()> {
        println!("🦞 Claw Interactive Chat");
        println!("   Type 'exit' or Ctrl+C to quit");
        println!("   Type '/status' for agent status");
        println!("   Type '/goals' to list goals");
        println!();

        let mut runtime = AgentRuntime::new(config.clone())?;

        // Register LLM providers — config file keys take priority, env vars are fallback
        let mut providers_registered = 0u32;
        if let Some(ref key) = config.services.anthropic_api_key {
            runtime.add_provider(Arc::new(
                claw_llm::anthropic::AnthropicProvider::new(key.clone()),
            ));
            providers_registered += 1;
        }
        if let Some(ref key) = config.services.openai_api_key {
            runtime.add_provider(Arc::new(
                claw_llm::openai::OpenAiProvider::new(key.clone()),
            ));
            providers_registered += 1;
        }

        if providers_registered == 0 {
            let model = &config.agent.model;
            let is_local = model.starts_with("ollama/") || model.starts_with("local/");
            if !is_local {
                eprintln!("⚠️  No LLM API keys found.");
                if model.starts_with("anthropic/") {
                    eprintln!("   Add to [services] in claw.toml:  anthropic_api_key = \"sk-ant-...\"");
                    eprintln!("   Or set env var: export ANTHROPIC_API_KEY=sk-ant-...");
                } else if model.starts_with("openai/") {
                    eprintln!("   Add to [services] in claw.toml:  openai_api_key = \"sk-...\"");
                    eprintln!("   Or set env var: export OPENAI_API_KEY=sk-...");
                } else {
                    eprintln!("   Add API keys to [services] in claw.toml or set ANTHROPIC_API_KEY / OPENAI_API_KEY.");
                }
                eprintln!();
            }
        }

        // Spawn the runtime in the background
        tokio::spawn(async move {
            if let Err(e) = runtime.run().await {
                error!(error = %e, "runtime exited with error");
            }
        });

        // Wait a moment for the runtime to register its handle
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let handle = claw_runtime::get_runtime_handle().await;
        let handle = match handle {
            Some(h) => h,
            None => {
                println!("❌ Failed to connect to runtime. Is the agent starting up?");
                return Ok(());
            }
        };

        let session_id: Option<String> = _session;

        // Interactive loop reading from stdin
        let stdin = tokio::io::stdin();
        let reader = tokio::io::BufReader::new(stdin);
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();

        loop {
            // Print prompt
            eprint!("\x1b[36myou>\x1b[0m ");
            use std::io::Write;
            std::io::stderr().flush().ok();

            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break, // EOF
                Err(_) => break,
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "exit" || trimmed == "quit" || trimmed == "/exit" {
                println!("👋 Goodbye!");
                break;
            }

            // Send message to agent via streaming so we can handle approval prompts
            match handle.chat_stream(trimmed.to_string(), session_id.clone()).await {
                Ok(mut rx) => {
                    let mut got_text = false;
                    while let Some(event) = rx.recv().await {
                        use claw_runtime::StreamEvent;
                        match event {
                            StreamEvent::TextDelta { content } => {
                                if !got_text {
                                    eprint!("\x1b[32mclaw>\x1b[0m ");
                                    got_text = true;
                                }
                                print!("{}", content);
                                std::io::stdout().flush().ok();
                            }
                            StreamEvent::Thinking { content } => {
                                eprint!("\x1b[90m💭 {}\x1b[0m", content);
                            }
                            StreamEvent::ToolCall { name, id: _, .. } => {
                                eprintln!("\x1b[33m🔧 Calling tool: {}\x1b[0m", name);
                            }
                            StreamEvent::ToolResult { id: _, content, is_error, .. } => {
                                if is_error {
                                    eprintln!("\x1b[31m   ❌ {}\x1b[0m", truncate_output(&content, 200));
                                } else {
                                    eprintln!("\x1b[90m   ✓ {}\x1b[0m", truncate_output(&content, 200));
                                }
                            }
                            StreamEvent::ApprovalRequired { id, tool_name, tool_args, reason, risk_level } => {
                                println!();
                                println!("\x1b[33m⚠️  APPROVAL REQUIRED\x1b[0m");
                                println!("   🔧 Tool: \x1b[1m{}\x1b[0m", tool_name);
                                println!("   ⚡ Risk: {}/10", risk_level);
                                println!("   📋 Reason: {}", reason);
                                let args_pretty = serde_json::to_string_pretty(&tool_args)
                                    .unwrap_or_else(|_| tool_args.to_string());
                                let args_short = truncate_output(&args_pretty, 300);
                                println!("\x1b[90m   {}\x1b[0m", args_short);
                                println!();
                                eprint!("\x1b[33m   Approve? [y/n]>\x1b[0m ");
                                std::io::stderr().flush().ok();

                                // Read the approval decision
                                let decision = lines.next_line().await;
                                match decision {
                                    Ok(Some(ans)) => {
                                        let ans = ans.trim().to_lowercase();
                                        if let Ok(uuid) = id.parse::<uuid::Uuid>() {
                                            if ans == "y" || ans == "yes" || ans == "approve" {
                                                match handle.approve(uuid).await {
                                                    Ok(()) => eprintln!("\x1b[32m   ✅ Approved\x1b[0m"),
                                                    Err(e) => eprintln!("\x1b[31m   ❌ {}\x1b[0m", e),
                                                }
                                            } else {
                                                match handle.deny(uuid).await {
                                                    Ok(()) => eprintln!("\x1b[31m   ❌ Denied\x1b[0m"),
                                                    Err(e) => eprintln!("\x1b[31m   ❌ {}\x1b[0m", e),
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        // EOF or error — deny by default
                                        if let Ok(uuid) = id.parse::<uuid::Uuid>() {
                                            let _ = handle.deny(uuid).await;
                                            eprintln!("\x1b[31m   ❌ Denied (no input)\x1b[0m");
                                        }
                                    }
                                }
                            }
                            StreamEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                                eprintln!("\n\x1b[90m   [{} in / {} out, ${:.4}]\x1b[0m",
                                    input_tokens, output_tokens, cost_usd);
                            }
                            StreamEvent::Error { message } => {
                                println!("\x1b[31m❌ Error: {}\x1b[0m", message);
                            }
                            StreamEvent::Done => {
                                if got_text {
                                    println!(); // newline after streaming text
                                }
                            }
                            StreamEvent::Session { .. } => {}
                        }
                    }
                }
                Err(e) => {
                    println!("\x1b[31m❌ {}\x1b[0m", e);
                }
            }
            println!();
        }

        Ok(())
    }

    async fn cmd_status(config: claw_config::ClawConfig) -> claw_core::Result<()> {
        let listen = &config.server.listen;
        println!("Checking status at http://{}...", listen);

        let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();
        match client
            .get(format!("http://{}/api/v1/status", listen))
            .send()
            .await
        {
            Ok(resp) => {
                let data: serde_json::Value = resp.json().await.map_err(|e| {
                    claw_core::ClawError::Agent(e.to_string())
                })?;
                println!("{}", serde_json::to_string_pretty(&data).unwrap());
            }
            Err(_) => {
                println!("❌ Agent is not running at {}", listen);
            }
        }
        Ok(())
    }

    fn cmd_config(config: claw_config::ClawConfig, json: bool) -> claw_core::Result<()> {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&config)
                    .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?
            );
        } else {
            println!(
                "{}",
                toml::to_string_pretty(&config)
                    .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?
            );
        }
        Ok(())
    }

    async fn cmd_plugin(
        config: claw_config::ClawConfig,
        action: PluginAction,
    ) -> claw_core::Result<()> {
        let host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
        let registry = claw_plugin::PluginRegistry::new(&config.plugins.registry_url);

        match action {
            PluginAction::List => {
                let plugins = host.loaded();
                if plugins.is_empty() {
                    println!("No plugins installed.");
                } else {
                    for p in plugins {
                        println!("  {} v{} — {}", p.plugin.name, p.plugin.version, p.plugin.description);
                    }
                }
            }
            PluginAction::Install { name, version } => {
                registry
                    .install(&name, version.as_deref(), &config.plugins.plugin_dir)
                    .await?;
                println!("✅ Installed {}", name);
            }
            PluginAction::Uninstall { name } => {
                let mut host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
                host.uninstall(&name)?;
                println!("✅ Uninstalled {}", name);
            }
            PluginAction::Search { query } => {
                match registry.search(&query).await {
                    Ok(results) => {
                        for r in results {
                            println!("  {} v{} — {} ({} downloads)", r.name, r.version, r.description, r.downloads);
                        }
                    }
                    Err(e) => {
                        println!("Search failed: {}", e);
                    }
                }
            }
            PluginAction::Info { name } => {
                let mut host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
                let _ = host.discover();
                match host.get_manifest(&name) {
                    Some(manifest) => {
                        println!("\x1b[1m{}\x1b[0m v{}", manifest.plugin.name, manifest.plugin.version);
                        println!("  {}", manifest.plugin.description);
                        if !manifest.plugin.authors.is_empty() {
                            println!("  Authors: {}", manifest.plugin.authors.join(", "));
                        }
                        if let Some(ref license) = manifest.plugin.license {
                            println!("  License: {}", license);
                        }
                        if let Some(ref homepage) = manifest.plugin.homepage {
                            println!("  Homepage: {}", homepage);
                        }
                        if let Some(ref checksum) = manifest.plugin.checksum {
                            println!("  Checksum: {}", &checksum[..checksum.len().min(16)]);
                        }
                        // Capabilities
                        let caps = &manifest.capabilities;
                        if !caps.network.is_empty() || !caps.filesystem.is_empty() || caps.shell {
                            println!("\n  \x1b[1mCapabilities:\x1b[0m");
                            if !caps.network.is_empty() {
                                println!("    Network: {}", caps.network.join(", "));
                            }
                            if !caps.filesystem.is_empty() {
                                println!("    Filesystem: {}", caps.filesystem.join(", "));
                            }
                            if caps.shell {
                                println!("    Shell: yes");
                            }
                        }
                        // Tools
                        if !manifest.tools.is_empty() {
                            println!("\n  \x1b[1mTools ({}):\x1b[0m", manifest.tools.len());
                            for tool in &manifest.tools {
                                let risk = if tool.risk_level > 0 {
                                    format!(" [risk={}]", tool.risk_level)
                                } else {
                                    String::new()
                                };
                                let mutating = if tool.is_mutating { " ✏️" } else { "" };
                                println!("    {}{}{} — {}", tool.name, mutating, risk, tool.description);
                            }
                        }
                    }
                    None => {
                        println!("Plugin '{}' not found. Is it installed?", name);
                    }
                }
            }
            PluginAction::Create { name } => {
                Self::scaffold_plugin(&name, &config.plugins.plugin_dir)?;
            }
        }
        Ok(())
    }

    /// Scaffold a new plugin project with Cargo.toml, src/lib.rs, and plugin.toml.
    fn scaffold_plugin(name: &str, plugin_dir: &Path) -> claw_core::Result<()> {
        let project_dir = plugin_dir.join(name);
        if project_dir.exists() {
            return Err(claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("directory already exists: {}", project_dir.display()),
            });
        }

        std::fs::create_dir_all(project_dir.join("src"))?;

        // plugin.toml
        let manifest = format!(
            r#"[plugin]
name = "{name}"
version = "0.1.0"
description = "A Claw plugin"
authors = []

[capabilities]
# network = ["https://example.com/*"]
# filesystem = ["/tmp/{name}/*"]
# shell = false

[[tools]]
name = "hello"
description = "Say hello"
parameters = {{ "type": "object", "properties": {{ "name": {{ "type": "string" }} }} }}
risk_level = 0
is_mutating = false
"#,
            name = name
        );
        std::fs::write(project_dir.join("plugin.toml"), manifest)?;

        // Cargo.toml for the plugin crate
        let cargo_toml = format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
            name = name
        );
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

        // src/lib.rs — minimal plugin implementation
        let lib_rs = r##"//! Claw plugin — compiled to WebAssembly.
//!
//! Build with: cargo build --target wasm32-unknown-unknown --release

use std::alloc::{alloc, Layout};

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { alloc(layout) as u32 }
}

/// Main entry point — the host calls this with a JSON input.
/// Returns a packed u64: (result_ptr << 32) | result_len
#[unsafe(no_mangle)]
pub extern "C" fn claw_invoke(ptr: u32, len: u32) -> u64 {
    // Read the input JSON from host-provided memory
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let input: serde_json::Value = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(e) => return write_response(&format!(r#"{{"error":"bad input: {}"}}"#, e)),
    };

    let tool = input["tool"].as_str().unwrap_or("");
    let args = &input["arguments"];

    // Dispatch to tool implementations
    let result = match tool {
        "hello" => {
            let name = args["name"].as_str().unwrap_or("world");
            format!(r#"{{"result":"Hello, {}!"}}"#, name)
        }
        _ => format!(r#"{{"error":"unknown tool: {}"}}"#, tool),
    };

    write_response(&result)
}

fn write_response(json: &str) -> u64 {
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ((ptr as u64) << 32) | (bytes.len() as u64)
}
"##;
        std::fs::write(project_dir.join("src").join("lib.rs"), lib_rs)?;

        println!("✅ Created plugin scaffold at {}", project_dir.display());
        println!("   Build: cd {} && cargo build --target wasm32-unknown-unknown --release", project_dir.display());
        println!("   Then copy the .wasm file alongside plugin.toml");

        Ok(())
    }

    async fn cmd_skill(
        config: claw_config::ClawConfig,
        action: SkillAction,
        config_path: &Path,
    ) -> claw_core::Result<()> {
        // Resolve skills_dir relative to config directory (e.g. ~/.claw/skills)
        let config_dir = config_path.parent().unwrap_or(Path::new("."));
        let skills_dir = if config.plugins.plugin_dir.is_absolute() {
            config.plugins.plugin_dir.parent()
                .unwrap_or(Path::new("."))
                .join("skills")
        } else {
            config_dir.join("skills")
        };

        let mut registry = claw_skills::SkillRegistry::new_single(&skills_dir);
        let _ = registry.discover();

        match action {
            SkillAction::List => {
                let skills = registry.list();
                if skills.is_empty() {
                    println!("No skills found in {}", skills_dir.display());
                    println!("  Create one with: claw skill create <name>");
                } else {
                    println!("\x1b[1mAvailable Skills ({}):\x1b[0m\n", skills.len());
                    for s in skills {
                        let tags = if s.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", s.tags.join(", "))
                        };
                        println!("  \x1b[36m{}\x1b[0m v{}{}", s.name, s.version, tags);
                        println!("    {}", s.description);
                        println!("    File: {}", s.file_path.display());
                        println!();
                    }
                }
            }
            SkillAction::Show { name } => {
                match registry.get(&name) {
                    Some(skill) => {
                        println!("\x1b[1m{}\x1b[0m v{}", skill.name, skill.version);
                        println!("  {}", skill.description);
                        if let Some(ref author) = skill.author {
                            println!("  Author: {}", author);
                        }
                        if !skill.tags.is_empty() {
                            println!("  Tags: {}", skill.tags.join(", "));
                        }
                        println!("  File: {}", skill.file_path.display());

                        println!("\n  \x1b[1mInstructions:\x1b[0m");
                        for line in skill.body.lines() {
                            println!("    {}", line);
                        }
                    }
                    None => {
                        println!("Skill '{}' not found.", name);
                    }
                }
            }
            SkillAction::Run { name, param: _ } => {
                match registry.get(&name) {
                    Some(skill) => {
                        println!("📖 Skill '{}' — SKILL.md instructions:\n", name);
                        println!("{}", skill.body);
                        println!("\n\x1b[33mNote:\x1b[0m Skills are now prompt-injected instructions.");
                        println!("The LLM reads these instructions and uses built-in tools to execute them.");
                        println!("Start the agent with 'claw start' and ask it to use this skill.");
                    }
                    None => {
                        println!("Skill '{}' not found.", name);
                    }
                }
            }
            SkillAction::Create { name } => {
                let skill_dir = skills_dir.join(&name);
                if skill_dir.exists() {
                    return Err(claw_core::ClawError::Agent(
                        format!("Skill '{}' already exists at {}", name, skill_dir.display()),
                    ));
                }

                std::fs::create_dir_all(&skill_dir)?;
                let skill_path = skill_dir.join("SKILL.md");

                let template = format!(
                    r#"---
name: {name}
description: Describe what this skill does
version: 1.0.0
tags: []
---

# {name}

## When to use this skill

Describe when this skill should be activated.

## Instructions

1. First, do this using `shell_exec`
2. Then check the result
3. Finally, report back to the user

## Notes

- Add any important notes or caveats here
- Reference files in this skill directory with {{baseDir}}
"#,
                    name = name
                );

                std::fs::write(&skill_path, template)?;
                println!("✅ Created skill template at {}", skill_path.display());
                println!("   Edit the SKILL.md, then start the agent — it will discover the skill automatically.");
            }
            SkillAction::Delete { name } => {
                let skill_dir = skills_dir.join(&name);
                if skill_dir.exists() {
                    std::fs::remove_dir_all(&skill_dir)?;
                    println!("✅ Deleted skill directory '{}'", skill_dir.display());
                } else {
                    // Try removing from registry
                    if registry.remove(&name) {
                        println!("✅ Removed skill '{}' from registry", name);
                    } else {
                        println!("Skill '{}' not found.", name);
                    }
                }
            }
            SkillAction::Push { name } => {
                // Resolve hub URL — services.hub_url is required for push/pull/search
                let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                    claw_core::ClawError::Agent(
                        "No hub_url configured. Set services.hub_url in claw.toml or run 'claw hub serve' to host your own hub.".into(),
                    )
                })?;
                let hub_url = hub_url.trim_end_matches('/');

                // Read the local SKILL.md and push it to the hub
                let skill = registry.get(&name).ok_or_else(|| {
                    claw_core::ClawError::Agent(
                        format!("Skill '{}' not found locally. Use 'claw skill list' to see available skills.", name),
                    )
                })?;

                // Read the raw SKILL.md file
                let skill_content = std::fs::read_to_string(&skill.file_path)?;

                let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();
                let url = format!("{}/api/v1/hub/skills", hub_url);

                let resp = client.post(&url)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({ "skill_content": skill_content }))
                    .send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach hub at {} — is it running? ({})", hub_url, e
                    ))
                })?;

                if resp.status().is_success() {
                    let data: serde_json::Value = resp.json().await.map_err(|e| {
                        claw_core::ClawError::Agent(e.to_string())
                    })?;
                    println!("✅ Published '{}' v{} to Skills Hub at {}",
                        data["name"].as_str().unwrap_or(&name),
                        data["version"].as_str().unwrap_or("?"),
                        hub_url);
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(claw_core::ClawError::Agent(
                        format!("Hub returned {}: {}", status, body),
                    ));
                }
            }
            SkillAction::Pull { name } => {
                let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                    claw_core::ClawError::Agent(
                        "No hub_url configured. Set services.hub_url in claw.toml.".into(),
                    )
                })?;
                let hub_url = hub_url.trim_end_matches('/');

                let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();
                let url = format!("{}/api/v1/hub/skills/{}/pull", hub_url, name);

                let resp = client.post(&url).send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach hub at {} — is it running? ({})", hub_url, e
                    ))
                })?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(claw_core::ClawError::Agent(
                        format!("Hub returned {}: {}", status, body),
                    ));
                }

                let data: serde_json::Value = resp.json().await.map_err(|e| {
                    claw_core::ClawError::Agent(e.to_string())
                })?;

                let skill_content = data["skill_content"].as_str().unwrap_or("");
                let version = data["version"].as_str().unwrap_or("?");
                let skill_name = data["name"].as_str().unwrap_or(&name);

                // Save to local skills directory as SKILL.md in a subdirectory
                let skill_dir = skills_dir.join(skill_name);
                std::fs::create_dir_all(&skill_dir)?;
                let path = skill_dir.join("SKILL.md");
                std::fs::write(&path, skill_content)?;
                println!("✅ Pulled '{}' v{} from {} → {}", skill_name, version, hub_url, path.display());
            }
            SkillAction::Search { query, tag } => {
                let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                    claw_core::ClawError::Agent(
                        "No hub_url configured. Set services.hub_url in claw.toml.".into(),
                    )
                })?;
                let hub_url = hub_url.trim_end_matches('/');

                let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();
                let mut url = format!("{}/api/v1/hub/skills/search?q={}", hub_url, query);
                if let Some(ref t) = tag {
                    url.push_str(&format!("&tag={}", t));
                }

                let resp = client.get(&url).send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach hub at {} — is it running? ({})", hub_url, e
                    ))
                })?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    return Err(claw_core::ClawError::Agent(
                        format!("Hub returned {}", status),
                    ));
                }

                let data: serde_json::Value = resp.json().await.map_err(|e| {
                    claw_core::ClawError::Agent(e.to_string())
                })?;

                let skills = data["skills"].as_array();
                match skills {
                    Some(skills) if !skills.is_empty() => {
                        println!("🔍 Found {} skill(s) matching '{}' on {}:\n", skills.len(), query, hub_url);
                        for s in skills {
                            let name = s["name"].as_str().unwrap_or("?");
                            let desc = s["description"].as_str().unwrap_or("");
                            let ver = s["version"].as_str().unwrap_or("?");
                            let dl = s["downloads"].as_u64().unwrap_or(0);
                            let tags: Vec<&str> = s["tags"].as_array()
                                .map(|t| t.iter().filter_map(|v| v.as_str()).collect())
                                .unwrap_or_default();

                            println!("  📦 {} v{} (⬇ {})", name, ver, dl);
                            if !desc.is_empty() {
                                println!("     {}", desc);
                            }
                            if !tags.is_empty() {
                                println!("     tags: {}", tags.join(", "));
                            }
                            println!();
                        }
                        println!("Pull a skill with: claw skill pull <name>");
                    }
                    _ => {
                        println!("No skills found matching '{}' on {}", query, hub_url);
                    }
                }
            }
        }
        Ok(())
    }

    async fn cmd_hub(action: HubAction) -> claw_core::Result<()> {
        match action {
            HubAction::Serve { listen, db } => {
                let db_path = match db {
                    Some(p) => std::path::PathBuf::from(p),
                    None => dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".claw")
                        .join("hub.db"),
                };

                if let Some(parent) = db_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }

                println!("🦞 Claw Skills Hub");
                println!("   Database: {}", db_path.display());
                println!("   Listening: http://{}", listen);
                println!();
                println!("   Remote agents should set in their claw.toml:");
                println!("   [services]");
                println!("   hub_url = \"http://{}\"", listen);
                println!();

                let router = claw_server::hub::standalone_hub_router(&db_path)
                    .map_err(|e| claw_core::ClawError::Agent(e))?;

                let listener = tokio::net::TcpListener::bind(&listen).await.map_err(|e| {
                    claw_core::ClawError::Agent(format!("Failed to bind {}: {}", listen, e))
                })?;

                println!("✅ Hub server started — press Ctrl+C to stop\n");

                axum::serve(listener, router).await.map_err(|e| {
                    claw_core::ClawError::Agent(format!("Hub server error: {}", e))
                })?;
            }
        }
        Ok(())
    }

    async fn cmd_mesh(config: claw_config::ClawConfig, action: MeshAction) -> claw_core::Result<()> {
        let listen = &config.server.listen;
        let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();

        let build_req = |url: &str| -> reqwest::RequestBuilder {
            let mut req = client.get(url);
            if let Some(ref key) = config.server.api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req
        };

        match action {
            MeshAction::Status => {
                let url = format!("http://{}/api/v1/mesh/status", listen);
                let resp = build_req(&url).send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach agent at {} — is it running? ({})", listen, e
                    ))
                })?;

                if !resp.status().is_success() {
                    return Err(claw_core::ClawError::Agent(format!(
                        "Server returned {}", resp.status()
                    )));
                }

                let data: serde_json::Value = resp.json().await.map_err(|e| {
                    claw_core::ClawError::Agent(e.to_string())
                })?;

                println!("🕸️  Mesh Status\n");
                println!("   Enabled:      {}", data["enabled"].as_bool().unwrap_or(false));
                println!("   Running:      {}", data["running"].as_bool().unwrap_or(false));
                println!("   Peer ID:      {}", data["peer_id"].as_str().unwrap_or("—"));
                println!("   Peers:        {}", data["peer_count"].as_u64().unwrap_or(0));
                println!("   Listen:       {}", data["listen"].as_str().unwrap_or("—"));
                println!("   mDNS:         {}", data["mdns"].as_bool().unwrap_or(false));
                if let Some(caps) = data["capabilities"].as_array() {
                    let cap_strs: Vec<&str> = caps.iter().filter_map(|c| c.as_str()).collect();
                    println!("   Capabilities: {}", if cap_strs.is_empty() { "none".to_string() } else { cap_strs.join(", ") });
                }
            }
            MeshAction::Peers => {
                let url = format!("http://{}/api/v1/mesh/peers", listen);
                let resp = build_req(&url).send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach agent at {} — is it running? ({})", listen, e
                    ))
                })?;

                if !resp.status().is_success() {
                    return Err(claw_core::ClawError::Agent(format!(
                        "Server returned {}", resp.status()
                    )));
                }

                let data: serde_json::Value = resp.json().await.map_err(|e| {
                    claw_core::ClawError::Agent(e.to_string())
                })?;

                let peers = data["peers"].as_array();
                let count = data["count"].as_u64().unwrap_or(0);

                if count == 0 {
                    println!("🕸️  No peers connected.");
                    if !config.mesh.enabled {
                        println!("\n   Mesh networking is disabled. Enable it in claw.toml:");
                        println!("   [mesh]");
                        println!("   enabled = true");
                    }
                } else {
                    println!("🕸️  Mesh Peers ({} connected)\n", count);
                    if let Some(peers) = peers {
                        for peer in peers {
                            let id = peer["peer_id"].as_str().unwrap_or("?");
                            let host = peer["hostname"].as_str().unwrap_or("?");
                            let os = peer["os"].as_str().unwrap_or("?");
                            let caps: Vec<&str> = peer["capabilities"]
                                .as_array()
                                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                                .unwrap_or_default();
                            println!("   📡 {}", id);
                            println!("      Host: {} ({})", host, os);
                            println!("      Capabilities: {}", if caps.is_empty() { "none".to_string() } else { caps.join(", ") });
                            println!();
                        }
                    }
                }
            }
            MeshAction::Send { peer_id, message } => {
                let url = format!("http://{}/api/v1/mesh/send", listen);
                let body = serde_json::json!({
                    "peer_id": peer_id,
                    "message": message,
                });

                let mut req = client.post(&url).json(&body);
                if let Some(ref key) = config.server.api_key {
                    req = req.header("Authorization", format!("Bearer {}", key));
                }

                let resp = req.send().await.map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach agent at {} — is it running? ({})", listen, e
                    ))
                })?;

                if resp.status().is_success() {
                    println!("✅ Message sent to peer {}", peer_id);
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(claw_core::ClawError::Agent(format!(
                        "Failed to send message: {} — {}", status, body
                    )));
                }
            }
        }
        Ok(())
    }

    async fn cmd_channels(config: claw_config::ClawConfig, action: ChannelAction) -> claw_core::Result<()> {
        match action {
            ChannelAction::Status => {
                println!("\x1b[1m📡 Channel Status\x1b[0m\n");

                let cred_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".claw")
                    .join("credentials");

                // Check WhatsApp
                let wa_linked = cred_dir.join("whatsapp").join("creds.json").exists();
                let wa_configured = config.channels.iter().any(|(_, c)| c.channel_type == "whatsapp");
                if wa_configured || wa_linked {
                    let status = if wa_linked { "\x1b[32m● linked\x1b[0m" } else { "\x1b[31m○ not linked\x1b[0m" };
                    let dm_policy = config.channels.iter()
                        .find(|(_, c)| c.channel_type == "whatsapp")
                        .map(|(_, c)| c.dm_policy.as_str())
                        .unwrap_or("pairing");
                    println!("   📱 WhatsApp:  {} (dm: {})", status, dm_policy);
                    if !wa_linked {
                        println!("      \x1b[90m→ Run: claw channels login whatsapp\x1b[0m");
                    }
                }

                // Check Telegram
                let tg_configured = config.channels.iter().any(|(_, c)| c.channel_type == "telegram");
                if tg_configured {
                    let has_token = config.channels.iter()
                        .find(|(_, c)| c.channel_type == "telegram")
                        .and_then(|(_, c)| c.settings.get("token"))
                        .is_some();
                    let status = if has_token { "\x1b[32m● configured\x1b[0m" } else { "\x1b[33m○ no token\x1b[0m" };
                    println!("   🤖 Telegram:  {}", status);
                }

                // Check Discord
                let dc_configured = config.channels.iter().any(|(_, c)| c.channel_type == "discord");
                if dc_configured {
                    let has_token = config.channels.iter()
                        .find(|(_, c)| c.channel_type == "discord")
                        .and_then(|(_, c)| c.settings.get("token"))
                        .is_some();
                    let status = if has_token { "\x1b[32m● configured\x1b[0m" } else { "\x1b[33m○ no token\x1b[0m" };
                    println!("   💬 Discord:   {}", status);
                }

                // Check Signal
                let sig_configured = config.channels.iter().any(|(_, c)| c.channel_type == "signal");
                if sig_configured {
                    let cli_ok = claw_channels::signal::SignalChannel::is_signal_cli_available();
                    let status = if cli_ok { "\x1b[32m● signal-cli found\x1b[0m" } else { "\x1b[31m○ signal-cli not found\x1b[0m" };
                    println!("   🔒 Signal:    {}", status);
                }

                // Check Slack
                let sl_configured = config.channels.iter().any(|(_, c)| c.channel_type == "slack");
                if sl_configured {
                    println!("   📎 Slack:     \x1b[32m● configured\x1b[0m");
                }

                // WebChat is always available
                let wc_configured = config.channels.iter().any(|(_, c)| c.channel_type == "webchat");
                if wc_configured {
                    println!("   🌐 WebChat:   \x1b[32m● enabled\x1b[0m (http://{})", config.server.listen);
                }

                let total = config.channels.len();
                if total == 0 {
                    println!("   No channels configured.");
                    println!("\n   Run 'claw setup' to configure channels, or add them manually:");
                    println!("   claw channels login whatsapp   — Scan QR to link WhatsApp");
                    println!("   claw set channels.telegram.type telegram");
                }

                println!();
            }
            ChannelAction::Login { channel, account: _, force } => {
                match channel.to_lowercase().as_str() {
                    "whatsapp" | "wa" => {
                        println!("\x1b[1m📱 WhatsApp — Link Device\x1b[0m\n");

                        let cred_dir = dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".claw")
                            .join("credentials")
                            .join("whatsapp");

                        let _ = std::fs::create_dir_all(&cred_dir);

                        let already_linked = cred_dir.join("creds.json").exists();
                        if already_linked && !force {
                            println!("   ✅ WhatsApp is already linked.");
                            println!("   Credentials: {}", cred_dir.display());
                            println!("\n   To re-link, run: claw channels login whatsapp --force");
                            return Ok(());
                        }

                        // Check Node.js
                        let node_ok = std::process::Command::new("node")
                            .arg("--version")
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);

                        if !node_ok {
                            println!("   ❌ Node.js not found. WhatsApp requires Node.js ≥ 18.");
                            println!("   Install from: https://nodejs.org/");
                            return Ok(());
                        }

                        // Install bridge if needed
                        if !claw_channels::whatsapp::WhatsAppChannel::is_bridge_installed() {
                            println!("   ⏳ Installing WhatsApp bridge (first time only)...");
                            claw_channels::whatsapp::WhatsAppChannel::install_bridge()?;
                            println!("   ✅ Bridge installed\n");
                        }

                        println!("   Starting WhatsApp bridge...\n");
                        println!("   📋 Instructions:");
                        println!("   1. Open WhatsApp on your phone");
                        println!("   2. Go to Settings → Linked Devices");
                        println!("   3. Tap 'Link a Device'");
                        println!("   4. Scan the QR code that appears below\n");
                        println!("   \x1b[33m⏳ Waiting for QR code from WhatsApp...\x1b[0m\n");

                        // Start the bridge and wait for QR + connection
                        let bridge_dir = claw_channels::whatsapp::WhatsAppChannel::bridge_dir();
                        let bridge_script = bridge_dir.join("bridge.js");

                        let mut child = std::process::Command::new("node")
                            .arg(&bridge_script)
                            .env("AUTH_DIR", cred_dir.to_string_lossy().to_string())
                            .env("BRIDGE_PORT", "0")
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::inherit())
                            .spawn()
                            .map_err(|e| claw_core::ClawError::Channel {
                                channel: "whatsapp".into(),
                                reason: format!("Failed to start bridge: {}", e),
                            })?;

                        let stdout = child.stdout.take().expect("stdout piped");
                        let reader = std::io::BufReader::new(stdout);
                        use std::io::BufRead;

                        let mut linked = false;
                        for line in reader.lines() {
                            let line = match line {
                                Ok(l) => l,
                                Err(_) => break,
                            };
                            let trimmed = line.trim();
                            if trimmed.is_empty() { continue; }

                            let event: serde_json::Value = match serde_json::from_str(trimmed) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            match event["type"].as_str().unwrap_or("") {
                                "qr" => {
                                    let qr_data = event["data"].as_str().unwrap_or("");
                                    match qrcode::QrCode::new(qr_data.as_bytes()) {
                                        Ok(code) => {
                                            let width = code.width();
                                            let colors: Vec<bool> = code.into_colors().into_iter()
                                                .map(|c| c == qrcode::Color::Dark).collect();
                                            let quiet = 1i32;
                                            let at = |x: i32, y: i32| -> bool {
                                                if x < 0 || y < 0 || x >= width as i32 || y >= width as i32 { false }
                                                else { colors[(y as usize) * width + (x as usize)] }
                                            };
                                            let total = width as i32 + quiet * 2;

                                            println!("   \x1b[1m📱 Scan this QR code with WhatsApp:\x1b[0m\n");
                                            let mut y = -quiet;
                                            while y < total - quiet {
                                                let mut row = String::from("   ");
                                                for x in -quiet..total - quiet {
                                                    let top = at(x, y);
                                                    let bot = at(x, y + 1);
                                                    row.push(match (top, bot) {
                                                        (true, true) => '█',
                                                        (true, false) => '▀',
                                                        (false, true) => '▄',
                                                        (false, false) => ' ',
                                                    });
                                                }
                                                println!("{}", row);
                                                y += 2;
                                            }
                                            println!();
                                            println!("   \x1b[33m⏳ Waiting for scan...\x1b[0m");
                                        }
                                        Err(e) => {
                                            eprintln!("   ⚠️  QR render failed: {}. Raw data: {}", e, qr_data);
                                        }
                                    }
                                }
                                "connected" => {
                                    let phone = event["phone"].as_str().unwrap_or("unknown");
                                    println!();
                                    println!("   \x1b[32m✅ WhatsApp linked successfully!\x1b[0m");
                                    println!("   Phone: {}", phone);
                                    println!("   Credentials: {}", cred_dir.display());
                                    println!();
                                    println!("   Start your agent: claw start");
                                    linked = true;
                                    break;
                                }
                                "disconnected" => {
                                    let reason = event["reason"].as_str().unwrap_or("unknown");
                                    if reason == "logged_out" {
                                        eprintln!("   ⚠️  Logged out during linking.");
                                        break;
                                    }
                                    // Otherwise it reconnects automatically
                                }
                                "error" => {
                                    let msg = event["message"].as_str().unwrap_or("unknown error");
                                    eprintln!("   ❌ Bridge error: {}", msg);
                                    break;
                                }
                                _ => {}
                            }
                        }

                        // Kill bridge after login
                        let _ = child.kill();
                        let _ = child.wait();

                        if !linked {
                            println!("\n   ⚠️  WhatsApp linking was not completed.");
                            println!("   Try again: claw channels login whatsapp --force");
                        }
                    }
                    "telegram" | "tg" => {
                        println!("\x1b[1m🤖 Telegram — Bot Setup\x1b[0m\n");
                        println!("   1. Open Telegram and message @BotFather");
                        println!("   2. Send /newbot and follow the prompts");
                        println!("   3. Copy the bot token\n");

                        use dialoguer::{Input, theme::ColorfulTheme};
                        let theme = ColorfulTheme::default();
                        let token: String = Input::with_theme(&theme)
                            .with_prompt("Telegram bot token")
                            .interact_text()
                            .unwrap_or_default();

                        if !token.is_empty() {
                            // Write to config
                            let config_path = claw_config::ConfigLoader::resolve_path(None);
                            if config_path.exists() {
                                let content = std::fs::read_to_string(&config_path)?;
                                let mut doc = content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {}", e))
                                })?;

                                doc["channels"]["telegram"]["type"] = toml_edit::value("telegram");
                                doc["channels"]["telegram"]["token"] = toml_edit::value(&token);

                                std::fs::write(&config_path, doc.to_string())?;
                                println!("\n   ✅ Telegram bot configured!");
                                println!("   Restart claw to activate: claw start");
                            }
                        }
                    }
                    "discord" | "dc" => {
                        println!("\x1b[1m💬 Discord — Bot Setup\x1b[0m\n");
                        println!("   1. Go to https://discord.com/developers/applications");
                        println!("   2. Create a new application → Bot → copy the token");
                        println!("   3. Enable required intents (Message Content, etc.)");
                        println!("   4. Invite to your server with the OAuth2 URL generator\n");

                        use dialoguer::{Input, theme::ColorfulTheme};
                        let theme = ColorfulTheme::default();
                        let token: String = Input::with_theme(&theme)
                            .with_prompt("Discord bot token")
                            .interact_text()
                            .unwrap_or_default();

                        if !token.is_empty() {
                            let config_path = claw_config::ConfigLoader::resolve_path(None);
                            if config_path.exists() {
                                let content = std::fs::read_to_string(&config_path)?;
                                let mut doc = content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {}", e))
                                })?;

                                doc["channels"]["discord"]["type"] = toml_edit::value("discord");
                                doc["channels"]["discord"]["token"] = toml_edit::value(&token);

                                std::fs::write(&config_path, doc.to_string())?;
                                println!("\n   ✅ Discord bot configured!");
                                println!("   Restart claw to activate: claw start");
                            }
                        }
                    }
                    "signal" => {
                        println!("\x1b[1m🔒 Signal — Setup\x1b[0m\n");
                        if claw_channels::signal::SignalChannel::is_signal_cli_available() {
                            println!("   ✅ signal-cli found");
                        } else {
                            println!("   ❌ signal-cli not found");
                            println!("   Install: brew install signal-cli (macOS)");
                            println!("   Or: https://github.com/AsamK/signal-cli/releases\n");
                            return Ok(());
                        }
                        println!("   Register your phone number:");
                        println!("   signal-cli -u +1234567890 register");
                        println!("   signal-cli -u +1234567890 verify <CODE>");
                        println!("\n   Then add to claw.toml:");
                        println!("   [channels.signal]");
                        println!("   type = \"signal\"");
                        println!("   phone = \"+1234567890\"");
                    }
                    "slack" => {
                        println!("\x1b[1m📎 Slack — Bot Setup\x1b[0m\n");
                        println!("   1. Go to https://api.slack.com/apps and create a new app");
                        println!("   2. Add bot scopes: chat:write, channels:history, im:history");
                        println!("   3. Install to workspace");
                        println!("   4. Copy the Bot Token (xoxb-...) and App Token (xapp-...)\n");

                        use dialoguer::{Input, theme::ColorfulTheme};
                        let theme = ColorfulTheme::default();
                        let bot_token: String = Input::with_theme(&theme)
                            .with_prompt("Slack bot token (xoxb-...)")
                            .interact_text()
                            .unwrap_or_default();

                        if !bot_token.is_empty() {
                            let config_path = claw_config::ConfigLoader::resolve_path(None);
                            if config_path.exists() {
                                let content = std::fs::read_to_string(&config_path)?;
                                let mut doc = content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {}", e))
                                })?;

                                doc["channels"]["slack"]["type"] = toml_edit::value("slack");
                                doc["channels"]["slack"]["bot_token"] = toml_edit::value(&bot_token);

                                std::fs::write(&config_path, doc.to_string())?;
                                println!("\n   ✅ Slack bot configured!");
                            }
                        }
                    }
                    other => {
                        eprintln!("❌ Unknown channel: '{}'", other);
                        eprintln!("   Supported: whatsapp, telegram, discord, signal, slack");
                    }
                }
            }
            ChannelAction::Logout { channel, account: _ } => {
                match channel.to_lowercase().as_str() {
                    "whatsapp" | "wa" => {
                        let wa = claw_channels::whatsapp::WhatsAppChannel::new(
                            "whatsapp".into(), None,
                        );
                        wa.logout()?;
                        println!("✅ WhatsApp session cleared. Re-link with: claw channels login whatsapp");
                    }
                    other => {
                        println!("Channel '{}' logout: removing config entry.", other);
                        println!("Edit claw.toml to fully remove the channel configuration.");
                    }
                }
            }
            ChannelAction::Pairing { channel } => {
                match channel.to_lowercase().as_str() {
                    "whatsapp" | "wa" => {
                        let wa = claw_channels::whatsapp::WhatsAppChannel::new(
                            "whatsapp".into(), None,
                        );
                        let requests = wa.load_pairing_requests();
                        if requests.is_empty() {
                            println!("No pending WhatsApp pairing requests.");
                        } else {
                            println!("\x1b[1mPending WhatsApp Pairing Requests:\x1b[0m\n");
                            for req in &requests {
                                println!("   Code: \x1b[1m{}\x1b[0m", req.code);
                                println!("   From: {} {}", req.sender,
                                    req.sender_name.as_deref().unwrap_or(""));
                                println!("   Time: {}", req.created_at);
                                println!("   Expires: {}", req.expires_at);
                                println!();
                            }
                            println!("   Approve: claw channels approve whatsapp <CODE>");
                            println!("   Deny:    claw channels deny whatsapp <CODE>");
                        }
                    }
                    other => {
                        println!("Pairing for '{}' — checking credential store...", other);
                        let cred_dir = dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".claw")
                            .join("credentials")
                            .join(other);
                        let pairing_file = cred_dir.join("pairing.json");
                        if pairing_file.exists() {
                            let data = std::fs::read_to_string(&pairing_file)?;
                            println!("{}", data);
                        } else {
                            println!("No pending pairing requests for '{}'.", other);
                        }
                    }
                }
            }
            ChannelAction::Approve { channel, code } => {
                match channel.to_lowercase().as_str() {
                    "whatsapp" | "wa" => {
                        let wa = claw_channels::whatsapp::WhatsAppChannel::new(
                            "whatsapp".into(), None,
                        );
                        match wa.approve_pairing(&code) {
                            Ok(sender) => {
                                println!("✅ Approved pairing for {} on WhatsApp", sender);
                            }
                            Err(e) => {
                                eprintln!("❌ {}", e);
                            }
                        }
                    }
                    other => {
                        println!("Pairing approval for '{}' with code '{}' — not yet implemented.", other, code);
                    }
                }
            }
            ChannelAction::Deny { channel, code } => {
                match channel.to_lowercase().as_str() {
                    "whatsapp" | "wa" => {
                        let wa = claw_channels::whatsapp::WhatsAppChannel::new(
                            "whatsapp".into(), None,
                        );
                        wa.deny_pairing(&code)?;
                        println!("❌ Denied pairing code {}", code);
                    }
                    _ => {
                        println!("Denied pairing code {} for {}", code, channel);
                    }
                }
            }
        }
        Ok(())
    }

    async fn cmd_logs(
        config: claw_config::ClawConfig,
        limit: usize,
        event_type: Option<String>,
        json: bool,
    ) -> claw_core::Result<()> {
        let listen = &config.server.listen;
        let client = reqwest::Client::builder().tcp_keepalive(None).build().unwrap_or_default();
        let url = format!("http://{}/api/v1/audit?limit={}", listen, limit);

        let mut req = client.get(&url);
        if let Some(ref key) = config.server.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await.map_err(|e| {
            claw_core::ClawError::Agent(format!(
                "Cannot reach agent at {} — is it running? ({})",
                listen, e
            ))
        })?;

        if !resp.status().is_success() {
            return Err(claw_core::ClawError::Agent(format!(
                "Server returned {}",
                resp.status()
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            claw_core::ClawError::Agent(e.to_string())
        })?;

        if json {
            println!("{}", serde_json::to_string_pretty(&data).unwrap());
            return Ok(());
        }

        let entries = data["audit_log"].as_array();
        let entries = match entries {
            Some(arr) => arr,
            None => {
                println!("No audit log entries.");
                return Ok(());
            }
        };

        // Filter by event_type if provided
        let filtered: Vec<&serde_json::Value> = if let Some(ref et) = event_type {
            entries.iter().filter(|e| {
                e["event_type"].as_str().map(|t| t.contains(et.as_str())).unwrap_or(false)
            }).collect()
        } else {
            entries.iter().collect()
        };

        if filtered.is_empty() {
            println!("No audit log entries{}",
                event_type.as_ref().map(|t| format!(" matching '{}'", t)).unwrap_or_default());
            return Ok(());
        }

        println!("\x1b[1mAudit Log\x1b[0m ({} entries)", filtered.len());
        println!("{}", "-".repeat(80));

        for entry in &filtered {
            let ts = entry["timestamp"].as_str().unwrap_or("");
            let etype = entry["event_type"].as_str().unwrap_or("unknown");
            let action = entry["action"].as_str().unwrap_or("");
            let details = entry["details"].as_str().unwrap_or("");

            // Color-code by event type
            let color = match etype {
                t if t.contains("denied") || t.contains("injection") => "\x1b[31m", // red
                t if t.contains("approval") => "\x1b[33m",   // yellow
                t if t.contains("tool") => "\x1b[36m",       // cyan
                t if t.contains("budget") => "\x1b[35m",     // magenta
                _ => "\x1b[37m",                              // default
            };

            println!("\x1b[90m{}\x1b[0m  {}{}\x1b[0m  {}", ts, color, etype, action);
            if !details.is_empty() {
                println!("   \x1b[90m{}\x1b[0m", truncate_output(details, 120));
            }
        }

        Ok(())
    }

    fn cmd_config_set(
        config_path: Option<PathBuf>,
        key: String,
        value: String,
    ) -> claw_core::Result<()> {
        let path = config_path.ok_or_else(|| {
            claw_core::ClawError::Config("No config file found. Run 'claw init' first.".into())
        })?;

        let content = std::fs::read_to_string(&path).map_err(|e| {
            claw_core::ClawError::Config(format!("Cannot read {}: {}", path.display(), e))
        })?;

        let mut doc = content.parse::<toml_edit::DocumentMut>().map_err(|e| {
            claw_core::ClawError::Config(format!("Invalid TOML in {}: {}", path.display(), e))
        })?;

        // Parse dot-notation key into table path, e.g. "agent.model" → ["agent", "model"]
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            return Err(claw_core::ClawError::Config("Empty key".into()));
        }

        // Navigate to the correct table, creating intermediate tables as needed
        let table_parts = &parts[..parts.len() - 1];
        let leaf_key = parts[parts.len() - 1];

        let mut table: &mut toml_edit::Item = doc.as_item_mut();
        for part in table_parts {
            // Ensure intermediate tables exist
            if table.get(part).is_none() {
                table[part] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            table = &mut table[part];
        }

        // Infer the value type: bool, integer, float, or string
        let toml_value = if value == "true" {
            toml_edit::value(true)
        } else if value == "false" {
            toml_edit::value(false)
        } else if let Ok(i) = value.parse::<i64>() {
            toml_edit::value(i)
        } else if let Ok(f) = value.parse::<f64>() {
            toml_edit::value(f)
        } else {
            toml_edit::value(&value)
        };

        let old_value = table.get(leaf_key).map(|v| v.to_string());
        table[leaf_key] = toml_value;

        std::fs::write(&path, doc.to_string()).map_err(|e| {
            claw_core::ClawError::Config(format!("Cannot write {}: {}", path.display(), e))
        })?;

        match old_value {
            Some(old) => println!("✅ {} = {} (was {})", key, value, old.trim()),
            None => println!("✅ {} = {} (new)", key, value),
        }

        Ok(())
    }

    fn cmd_doctor(config: claw_config::ClawConfig) -> claw_core::Result<()> {
        println!("🩺 Claw Doctor — Configuration Audit");
        println!();

        // Run structured validation
        let warnings = match config.validate() {
            Ok(w) => w,
            Err(e) => {
                println!("{}", e);
                return Ok(());
            }
        };

        let mut warn_count = 0;
        let mut info_count = 0;

        for w in &warnings {
            println!("  {}", w);
            match w.severity {
                claw_config::WarningSeverity::Warning => warn_count += 1,
                claw_config::WarningSeverity::Info => info_count += 1,
                _ => {}
            }
        }

        // Additional doctor-specific checks beyond basic validation
        let mut extra_ok = 0;

        // Check denylist
        if config.autonomy.tool_denylist.is_empty() {
            println!("  💡 autonomy.tool_denylist: no tools on denylist — consider blocking dangerous tools");
            info_count += 1;
        } else {
            extra_ok += 1;
        }

        // Check if API key is set (for any bind address)
        if config.server.api_key.is_some() {
            extra_ok += 1;
        }

        println!();
        let ok_total = extra_ok + (if warnings.is_empty() { 5 } else { 5 - warn_count - info_count });
        println!(
            "  ✅ {} checks passed, ⚠️  {} warnings, 💡 {} suggestions",
            ok_total, warn_count, info_count
        );

        Ok(())
    }

    fn cmd_init(local: bool) -> claw_core::Result<()> {
        let dir = if local {
            std::env::current_dir()?
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claw")
        };

        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("claw.toml");

        if config_path.exists() {
            println!("⚠️  {} already exists", config_path.display());
            println!("   Run 'claw setup' for an interactive configuration wizard.");
            return Ok(());
        }

        let _default_config = include_str!("../../claw-config/src/schema.rs");
        // Write a minimal config
        let minimal = r#"# 🦞 Claw Configuration
# See https://docs.claw.dev for full reference

[agent]
model = "anthropic/claude-sonnet-4-20250514"
# temperature = 0.7
# max_tokens = 8192
# thinking_level = "medium"

[autonomy]
level = 1  # 0=manual, 1=assisted, 2=supervised, 3=autonomous, 4=full_auto
daily_budget_usd = 10.0
# approval_threshold = 7
# proactive = false

[memory]
# db_path = "memory.db"
# vector_search = true

[server]
listen = "127.0.0.1:3700"
# web_ui = true
# api_key = "your-secret-key"

[services]
# anthropic_api_key = "sk-ant-..."   # or env: ANTHROPIC_API_KEY
# openai_api_key = "sk-..."          # or env: OPENAI_API_KEY
# brave_api_key = "..."              # or env: BRAVE_API_KEY
# hub_url = "http://your-hub-server:3800"     # Skills Hub — run 'claw hub serve' to host

# [channels.telegram]
# type = "telegram"
# token = "YOUR_BOT_TOKEN"

[channels.webchat]
type = "webchat"

[mesh]
enabled = true
listen = "/ip4/0.0.0.0/tcp/0"
mdns = true
capabilities = ["shell", "browser"]
# bootstrap_peers = ["/ip4/192.168.1.100/tcp/4001/p2p/PEER_ID"]
# psk = "shared-secret-for-mesh-encryption"

[logging]
level = "info"
# format = "pretty"
"#;

        std::fs::write(&config_path, minimal)?;
        println!("✅ Created {}", config_path.display());
        println!("   Edit it to configure your agent, then run: claw start");
        println!("   Or run 'claw setup' for an interactive wizard.");

        Ok(())
    }

    fn cmd_setup(local: bool, reset: bool, section: Option<String>) -> claw_core::Result<()> {
        use dialoguer::{Confirm, Input, MultiSelect, Select, theme::ColorfulTheme};

        let theme = ColorfulTheme::default();

        println!();
        println!("🦞 \x1b[1mClaw Setup Wizard\x1b[0m");
        println!("   Let's configure your autonomous AI agent!\n");
        println!("   ┌─────────────────────────────────────────────┐");
        println!("   │  Steps: Model → Channels → Autonomy →      │");
        println!("   │         Services → Mesh → Server            │");
        println!("   │                                             │");
        println!("   │  Tip: Use --section to jump to a section    │");
        println!("   │       e.g. claw setup --section channels    │");
        println!("   └─────────────────────────────────────────────┘\n");

        // ── 0. Config location ───────────────────────────────
        let dir = if local {
            std::env::current_dir()?
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claw")
        };
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("claw.toml");

        if config_path.exists() && !reset {
            let action = Select::with_theme(&theme)
                .with_prompt(format!("{} already exists", config_path.display()))
                .items(&[
                    "Keep existing config (edit individual sections)",
                    "Overwrite with a fresh config",
                    "Cancel setup",
                ])
                .default(0)
                .interact()
                .unwrap_or(2);

            match action {
                0 => {
                    // If no --section given, ask which section to edit
                    if section.is_none() {
                        let sections = &["model", "channels", "autonomy", "services", "mesh", "server"];
                        let idx = Select::with_theme(&theme)
                            .with_prompt("Which section to configure?")
                            .items(sections)
                            .default(1) // default to channels
                            .interact()
                            .unwrap_or(1);
                        println!("\n   Tip: You can also run: claw setup --section {}", sections[idx]);
                        // TODO: edit individual section in existing config
                        println!("   Section editing for '{}' coming soon. For now use: claw config set KEY VALUE", sections[idx]);
                        return Ok(());
                    }
                }
                1 => { /* continue with fresh config */ }
                _ => {
                    println!("   Setup cancelled.");
                    return Ok(());
                }
            }
        }

        if reset && config_path.exists() {
            std::fs::remove_file(&config_path)?;
            println!("   🗑  Old config removed.\n");
        }

        // Helper: should we run this section?
        let should_run = |name: &str| -> bool {
            match &section {
                Some(s) => s.eq_ignore_ascii_case(name),
                None => true,
            }
        };

        // ── Collected state ──────────────────────────────────
        let mut model = String::from("anthropic/claude-sonnet-4-20250514");
        let mut provider_prefix = "anthropic";
        let mut api_key_value = String::new();
        let mut env_var_name = "ANTHROPIC_API_KEY";
        let mut autonomy: usize = 2;
        let mut budget = String::from("10.0");

        // Channels state
        struct ChannelSetup {
            name: String,
            channel_type: String,
            token: String,
            dm_policy: String,
            allow_from: Vec<String>,
            enabled: bool,
        }
        let mut channels: Vec<ChannelSetup> = Vec::new();
        let mut link_now_channels: Vec<String> = Vec::new();

        // Services
        let mut brave_key = String::new();

        // Mesh
        let mut mesh_enabled = true;
        let mut mesh_capabilities = vec!["shell".to_string(), "browser".to_string()];
        let mut mesh_listen = "/ip4/0.0.0.0/tcp/0".to_string();

        // Server
        let mut listen = "127.0.0.1:3700".to_string();

        // ── 1. LLM Provider ─────────────────────────────────
        if should_run("model") {
            println!("\n\x1b[1m📡 Step 1/6 — LLM Provider\x1b[0m");
            println!("   Choose which AI model powers your agent.\n");

            let providers = &["Anthropic (Claude) — recommended", "OpenAI (GPT-4o, o3, etc.)", "Ollama (local, free, private)"];
            let provider_idx = Select::with_theme(&theme)
                .with_prompt("Which LLM provider?")
                .items(providers)
                .default(0)
                .interact()
                .unwrap_or(0);

            let info = match provider_idx {
                0 => ("anthropic", "anthropic/claude-sonnet-4-20250514", "ANTHROPIC_API_KEY", "sk-ant-..."),
                1 => ("openai", "openai/gpt-4o", "OPENAI_API_KEY", "sk-..."),
                2 => ("ollama", "ollama/llama3", "", ""),
                _ => ("anthropic", "anthropic/claude-sonnet-4-20250514", "ANTHROPIC_API_KEY", "sk-ant-..."),
            };
            provider_prefix = info.0;
            env_var_name = info.2;
            let _env_hint = info.3;

            model = Input::with_theme(&theme)
                .with_prompt("Model identifier")
                .default(info.1.into())
                .interact_text()
                .unwrap_or_else(|_| info.1.into());

            if !env_var_name.is_empty() {
                if let Ok(key) = std::env::var(env_var_name) {
                    api_key_value = key;
                    println!("   ✅ {} found in environment", env_var_name);
                } else {
                    println!("\n   You need an API key from your provider.");
                    if provider_prefix == "anthropic" {
                        println!("   Get one at: https://console.anthropic.com/settings/keys");
                    } else if provider_prefix == "openai" {
                        println!("   Get one at: https://platform.openai.com/api-keys");
                    }
                    let key: String = Input::with_theme(&theme)
                        .with_prompt(format!("{} API key (or press Enter to skip)", provider_prefix))
                        .allow_empty(true)
                        .interact_text()
                        .unwrap_or_default();
                    if !key.is_empty() {
                        api_key_value = key;
                        println!("   ✅ API key saved to config");
                    } else {
                        println!("   ⚠️  Skipped — set later: claw config set {}_api_key YOUR_KEY", provider_prefix);
                    }
                }
            }
        }

        // ── 2. Channels ─────────────────────────────────────
        if should_run("channels") {
            println!("\n\x1b[1m💬 Step 2/6 — Channels\x1b[0m");
            println!("   Connect your agent to messaging apps so you can talk to it");
            println!("   from your phone or desktop — just like chatting with a friend.\n");

            let channel_options = &[
                "WhatsApp  — scan QR code to connect (like WhatsApp Web)",
                "Telegram  — create a bot via @BotFather, paste the token",
                "Discord   — add a bot to your server with a bot token",
                "Slack     — install as a Slack app with a bot token",
                "Signal    — connect via signal-cli (privacy-focused)",
                "Web Chat  — built-in browser UI (always recommended)",
            ];
            let channel_types = &["whatsapp", "telegram", "discord", "slack", "signal", "webchat"];

            let defaults = vec![false, false, false, false, false, true]; // webchat on by default
            let selected = MultiSelect::with_theme(&theme)
                .with_prompt("Which channels do you want? (Space to select, Enter to confirm)")
                .items(channel_options)
                .defaults(&defaults)
                .interact()
                .unwrap_or_else(|_| vec![5]); // default to webchat only

            for idx in &selected {
                let ch_type = channel_types[*idx];
                let mut setup = ChannelSetup {
                    name: ch_type.to_string(),
                    channel_type: ch_type.to_string(),
                    token: String::new(),
                    dm_policy: "pairing".to_string(),
                    allow_from: Vec::new(),
                    enabled: true,
                };

                match ch_type {
                    "whatsapp" => {
                        println!("\n   \x1b[1m📱 WhatsApp Setup\x1b[0m");
                        println!("   Claw connects to WhatsApp the same way WhatsApp Web does.");
                        println!("   A QR code will appear — scan it with your phone to link.\n");

                        // DM policy
                        let policies = &[
                            "Pairing — require a pairing code before responding (most secure)",
                            "Allowlist — only respond to specific phone numbers",
                            "Open — respond to anyone who messages the bot",
                            "Disabled — don't accept any DMs",
                        ];
                        let policy_idx = Select::with_theme(&theme)
                            .with_prompt("Who can message your WhatsApp agent?")
                            .items(policies)
                            .default(0)
                            .interact()
                            .unwrap_or(0);

                        setup.dm_policy = match policy_idx {
                            0 => "pairing".into(),
                            1 => {
                                let nums: String = Input::with_theme(&theme)
                                    .with_prompt("Allowed phone numbers (comma-separated, e.g. +1234567890)")
                                    .interact_text()
                                    .unwrap_or_default();
                                setup.allow_from = nums.split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                "allowlist".into()
                            }
                            2 => "open".into(),
                            3 => "disabled".into(),
                            _ => "pairing".into(),
                        };

                        // Offer to link WhatsApp right now
                        let link_now = Confirm::with_theme(&theme)
                            .with_prompt("Link WhatsApp now? (scan QR code)")
                            .default(true)
                            .interact()
                            .unwrap_or(false);

                        if link_now {
                            link_now_channels.push("whatsapp".to_string());
                        } else {
                            println!("   📌 Link later: claw channels login whatsapp");
                        }
                    }
                    "telegram" => {
                        println!("\n   \x1b[1m🤖 Telegram Setup\x1b[0m");
                        println!("   1. Open Telegram and search for @BotFather");
                        println!("   2. Send /newbot and follow the prompts");
                        println!("   3. Copy the bot token and paste it below\n");

                        setup.token = Input::with_theme(&theme)
                            .with_prompt("Telegram bot token")
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();

                        if setup.token.is_empty() {
                            println!("   ⚠️  Skipped — add later: claw channels login telegram");
                            setup.enabled = false;
                        }
                    }
                    "discord" => {
                        println!("\n   \x1b[1m🎮 Discord Setup\x1b[0m");
                        println!("   1. Go to https://discord.com/developers/applications");
                        println!("   2. Create a new application → Bot section → Copy token");
                        println!("   3. Enable Message Content Intent under Privileged Intents");
                        println!("   4. Use OAuth2 URL Generator to invite bot to your server\n");

                        setup.token = Input::with_theme(&theme)
                            .with_prompt("Discord bot token")
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();

                        if setup.token.is_empty() {
                            println!("   ⚠️  Skipped — add later: claw channels login discord");
                            setup.enabled = false;
                        }
                    }
                    "slack" => {
                        println!("\n   \x1b[1m💼 Slack Setup\x1b[0m");
                        println!("   1. Go to https://api.slack.com/apps → Create New App");
                        println!("   2. Add Bot Token Scopes: chat:write, channels:history, im:history");
                        println!("   3. Install to workspace and copy the Bot User OAuth Token\n");

                        setup.token = Input::with_theme(&theme)
                            .with_prompt("Slack bot token (xoxb-...)")
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();

                        if setup.token.is_empty() {
                            println!("   ⚠️  Skipped — add later: claw channels login slack");
                            setup.enabled = false;
                        }
                    }
                    "signal" => {
                        println!("\n   \x1b[1m🔒 Signal Setup\x1b[0m");
                        println!("   Signal requires 'signal-cli' to be installed.");
                        println!("   Install: brew install signal-cli  (macOS)");
                        println!("            or see https://github.com/AsamK/signal-cli\n");

                        // Check if signal-cli is available
                        if claw_channels::signal::SignalChannel::is_signal_cli_available() {
                            println!("   ✅ signal-cli found!");
                        } else {
                            println!("   ⚠️  signal-cli not found. Install it before using Signal.");
                        }

                        let phone: String = Input::with_theme(&theme)
                            .with_prompt("Signal phone number (e.g. +1234567890)")
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();

                        if !phone.is_empty() {
                            setup.token = phone;
                            println!("   📌 You'll need to verify this number with signal-cli register/verify.");
                        } else {
                            println!("   ⚠️  Skipped — set up later: claw channels login signal");
                            setup.enabled = false;
                        }
                    }
                    "webchat" => {
                        println!("\n   \x1b[1m🌐 Web Chat\x1b[0m");
                        println!("   The built-in web UI will be available when you run 'claw start'.");
                        println!("   Access it at http://localhost:3700 in your browser.\n");
                    }
                    _ => {}
                }

                channels.push(setup);
            }

            if selected.is_empty() {
                println!("\n   No channels selected. You can add them later: claw channels login <channel>");
                // Add webchat by default
                channels.push(ChannelSetup {
                    name: "webchat".into(),
                    channel_type: "webchat".into(),
                    token: String::new(),
                    dm_policy: "open".into(),
                    allow_from: Vec::new(),
                    enabled: true,
                });
            }
        }

        // ── 3. Autonomy ─────────────────────────────────────
        if should_run("autonomy") {
            println!("\n\x1b[1m🤖 Step 3/6 — Autonomy Level\x1b[0m");
            println!("   How much freedom should your agent have?\n");

            let levels = &[
                "L0 Manual    — Agent only responds, never takes actions",
                "L1 Assisted  — Suggests actions, you approve each one",
                "L2 Supervised — Acts on safe tools, asks for risky ones  ← recommended",
                "L3 Autonomous — Acts freely within budget & guardrails",
                "L4 Full Auto  — No restrictions (use with caution!)",
            ];
            autonomy = Select::with_theme(&theme)
                .with_prompt("Autonomy level")
                .items(levels)
                .default(2)
                .interact()
                .unwrap_or(2);

            budget = Input::with_theme(&theme)
                .with_prompt("Daily LLM budget (USD)")
                .default("10.0".into())
                .interact_text()
                .unwrap_or_else(|_| "10.0".into());
        }

        // ── 4. Services ─────────────────────────────────────
        if should_run("services") {
            println!("\n\x1b[1m🔌 Step 4/6 — Services\x1b[0m");
            println!("   Optional API keys for extra capabilities.\n");

            let brave_enabled = Confirm::with_theme(&theme)
                .with_prompt("Enable web search? (Brave Search — free at https://api.search.brave.com/)")
                .default(false)
                .interact()
                .unwrap_or(false);

            if brave_enabled {
                brave_key = Input::with_theme(&theme)
                    .with_prompt("Brave Search API key")
                    .allow_empty(true)
                    .interact_text()
                    .unwrap_or_default();
            }
        }

        // ── 5. Mesh Networking ────────────────────────────────
        if should_run("mesh") {
            println!("\n\x1b[1m🕸️  Step 5/6 — Mesh Networking\x1b[0m");
            println!("   Connect multiple Claw agents on your LAN (or beyond) to");
            println!("   delegate tasks and share capabilities.\n");

            mesh_enabled = Confirm::with_theme(&theme)
                .with_prompt("Enable mesh networking?")
                .default(true)
                .interact()
                .unwrap_or(true);

            if mesh_enabled {
                let caps: String = Input::with_theme(&theme)
                    .with_prompt("Capabilities to advertise (comma-separated)")
                    .default("shell,browser".into())
                    .interact_text()
                    .unwrap_or_else(|_| "shell,browser".into());
                mesh_capabilities = caps.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                mesh_listen = Input::with_theme(&theme)
                    .with_prompt("Mesh listen address")
                    .default("/ip4/0.0.0.0/tcp/0".into())
                    .interact_text()
                    .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());
            }
        }

        // ── 6. Server ────────────────────────────────────────
        if should_run("server") {
            println!("\n\x1b[1m🌐 Step 6/6 — Server\x1b[0m");
            listen = Input::with_theme(&theme)
                .with_prompt("API server listen address")
                .default("127.0.0.1:3700".into())
                .interact_text()
                .unwrap_or_else(|_| "127.0.0.1:3700".into());
        }

        // ══════════════════════════════════════════════════════
        // Build config file
        // ══════════════════════════════════════════════════════
        let mut config = String::new();
        config.push_str("# 🦞 Claw Configuration\n");
        config.push_str("# Generated by 'claw setup'\n\n");

        // Agent section
        config.push_str("[agent]\n");
        config.push_str(&format!("model = \"{}\"\n", model));
        if provider_prefix == "anthropic" {
            config.push_str("# thinking_level = \"medium\"\n");
        }
        config.push_str("# temperature = 0.7\n");
        config.push_str("# max_tokens = 8192\n\n");

        // Autonomy section
        config.push_str("[autonomy]\n");
        config.push_str(&format!("level = {}\n", autonomy));
        config.push_str(&format!("daily_budget_usd = {}\n", budget));
        if autonomy >= 2 {
            config.push_str("approval_threshold = 7\n");
        }
        config.push('\n');

        // Memory section
        config.push_str("[memory]\n");
        config.push_str("# db_path = \"memory.db\"\n");
        config.push_str("# vector_search = true\n\n");

        // Server section
        config.push_str("[server]\n");
        config.push_str(&format!("listen = \"{}\"\n", listen));
        config.push_str("web_ui = true\n\n");

        // Services section
        config.push_str("[services]\n");
        if provider_prefix == "anthropic" {
            if !api_key_value.is_empty() {
                config.push_str(&format!("anthropic_api_key = \"{}\"\n", api_key_value));
            } else {
                config.push_str("# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n");
            }
            config.push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
        } else if provider_prefix == "openai" {
            config.push_str("# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n");
            if !api_key_value.is_empty() {
                config.push_str(&format!("openai_api_key = \"{}\"\n", api_key_value));
            } else {
                config.push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
            }
        } else {
            config.push_str("# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n");
            config.push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
        }
        if !brave_key.is_empty() {
            config.push_str(&format!("brave_api_key = \"{}\"\n", brave_key));
        } else {
            config.push_str("# brave_api_key = \"\"    # Get one free: https://api.search.brave.com/\n");
        }
        config.push_str("# hub_url = \"\"          # Skills Hub URL — run 'claw hub serve' to host one\n");
        config.push('\n');

        // ── Channels section ─────────────────────────────────
        config.push_str("# ── Channels ────────────────────────────────────────────────────\n");
        config.push_str("# Connect your agent to messaging apps.\n");
        config.push_str("# Manage channels:  claw channels status\n");
        config.push_str("# Login:            claw channels login whatsapp\n");
        config.push_str("# DM policies:      pairing | allowlist | open | disabled\n\n");

        for ch in &channels {
            if !ch.enabled && ch.token.is_empty() {
                config.push_str(&format!("# [channels.{}]    # run: claw channels login {}\n", ch.name, ch.name));
                config.push_str(&format!("# type = \"{}\"\n\n", ch.channel_type));
                continue;
            }

            config.push_str(&format!("[channels.{}]\n", ch.name));
            config.push_str(&format!("type = \"{}\"\n", ch.channel_type));

            if !ch.token.is_empty() {
                config.push_str(&format!("token = \"{}\"\n", ch.token));
            }

            if ch.channel_type == "whatsapp" || ch.channel_type == "signal" {
                config.push_str(&format!("dm_policy = \"{}\"\n", ch.dm_policy));
                if !ch.allow_from.is_empty() {
                    let nums: Vec<String> = ch.allow_from.iter().map(|n| format!("\"{}\"", n)).collect();
                    config.push_str(&format!("allow_from = [{}]\n", nums.join(", ")));
                }
            }

            config.push('\n');
        }

        // Mesh section
        config.push_str("# ── Mesh Network ────────────────────────────────────────────────\n\n");
        config.push_str("[mesh]\n");
        config.push_str(&format!("enabled = {}\n", mesh_enabled));
        config.push_str(&format!("listen = \"{}\"\n", mesh_listen));
        config.push_str("mdns = true\n");
        let caps_toml: Vec<String> = mesh_capabilities.iter().map(|c| format!("\"{}\"", c)).collect();
        config.push_str(&format!("capabilities = [{}]\n", caps_toml.join(", ")));
        config.push_str("# bootstrap_peers = [\"/ip4/192.168.1.100/tcp/4001/p2p/PEER_ID\"]\n");
        config.push_str("# psk = \"shared-secret-for-mesh-encryption\"\n\n");

        // Logging section
        config.push_str("[logging]\n");
        config.push_str("level = \"info\"\n");

        // Write the config file
        std::fs::write(&config_path, &config)?;

        // Create skills directory
        let skills_dir = dir.join("skills");
        std::fs::create_dir_all(&skills_dir)?;

        // Copy bundled skills if they don't exist (SKILL.md format)
        let bundled_skills: &[(&str, &str)] = &[
            ("plesk-server", include_str!("../../../skills/plesk-server/SKILL.md")),
            ("github", include_str!("../../../skills/github/SKILL.md")),
            ("docker", include_str!("../../../skills/docker/SKILL.md")),
            ("server-management", include_str!("../../../skills/server-management/SKILL.md")),
            ("coding", include_str!("../../../skills/coding/SKILL.md")),
            ("web-research", include_str!("../../../skills/web-research/SKILL.md")),
            ("system-admin", include_str!("../../../skills/system-admin/SKILL.md")),
            ("1password", include_str!("../../../skills/1password/SKILL.md")),
        ];
        for (name, content) in bundled_skills {
            let skill_dir = skills_dir.join(name);
            std::fs::create_dir_all(&skill_dir)?;
            let path = skill_dir.join("SKILL.md");
            if !path.exists() {
                std::fs::write(&path, content)?;
            }
        }

        // ══════════════════════════════════════════════════════
        // Inline channel linking (WhatsApp QR, etc.)
        // ══════════════════════════════════════════════════════
        if link_now_channels.contains(&"whatsapp".to_string()) {
            println!("\n\x1b[1m📱 Linking WhatsApp...\x1b[0m\n");

            let cred_dir = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claw")
                .join("credentials")
                .join("whatsapp");
            let _ = std::fs::create_dir_all(&cred_dir);

            // Check Node.js
            let node_ok = std::process::Command::new("node")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if !node_ok {
                println!("   ❌ Node.js not found. Install Node.js ≥ 18 first.");
                println!("   Then run: claw channels login whatsapp\n");
            } else {
                // Install bridge if needed
                if !claw_channels::whatsapp::WhatsAppChannel::is_bridge_installed() {
                    println!("   ⏳ Installing WhatsApp bridge (first time only)...");
                    match claw_channels::whatsapp::WhatsAppChannel::install_bridge() {
                        Ok(_) => println!("   ✅ Bridge installed\n"),
                        Err(e) => {
                            println!("   ❌ Bridge install failed: {}", e);
                            println!("   Try again later: claw channels login whatsapp\n");
                        }
                    }
                }

                if claw_channels::whatsapp::WhatsAppChannel::is_bridge_installed() {
                    println!("   Starting WhatsApp bridge...\n");
                    println!("   1. Open WhatsApp on your phone");
                    println!("   2. Go to Settings → Linked Devices");
                    println!("   3. Tap 'Link a Device'");
                    println!("   4. Scan the QR code that appears below\n");
                    println!("   \x1b[33m⏳ Waiting for QR code...\x1b[0m\n");

                    let bridge_dir = claw_channels::whatsapp::WhatsAppChannel::bridge_dir();
                    let bridge_script = bridge_dir.join("bridge.js");

                    if let Ok(mut child) = std::process::Command::new("node")
                        .arg(&bridge_script)
                        .env("AUTH_DIR", cred_dir.to_string_lossy().to_string())
                        .env("BRIDGE_PORT", "0")
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::inherit())
                        .spawn()
                    {
                        let stdout = child.stdout.take().expect("stdout piped");
                        let reader = std::io::BufReader::new(stdout);
                        use std::io::BufRead;

                        for line in reader.lines() {
                            let line = match line { Ok(l) => l, Err(_) => break };
                            let trimmed = line.trim();
                            if trimmed.is_empty() { continue; }

                            let event: serde_json::Value = match serde_json::from_str(trimmed) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            match event["type"].as_str().unwrap_or("") {
                                "qr" => {
                                    let qr_data = event["data"].as_str().unwrap_or("");
                                    if let Ok(code) = qrcode::QrCode::new(qr_data.as_bytes()) {
                                        let width = code.width();
                                        let colors: Vec<bool> = code.into_colors().into_iter()
                                            .map(|c| c == qrcode::Color::Dark).collect();
                                        let quiet = 1i32;
                                        let at = |x: i32, y: i32| -> bool {
                                            if x < 0 || y < 0 || x >= width as i32 || y >= width as i32 { false }
                                            else { colors[(y as usize) * width + (x as usize)] }
                                        };
                                        let total = width as i32 + quiet * 2;

                                        println!("   \x1b[1m📱 Scan this QR code with WhatsApp:\x1b[0m\n");
                                        let mut y = -quiet;
                                        while y < total - quiet {
                                            let mut row = String::from("   ");
                                            for x in -quiet..total - quiet {
                                                let top = at(x, y);
                                                let bot = at(x, y + 1);
                                                row.push(match (top, bot) {
                                                    (true, true) => '█',
                                                    (true, false) => '▀',
                                                    (false, true) => '▄',
                                                    (false, false) => ' ',
                                                });
                                            }
                                            println!("{}", row);
                                            y += 2;
                                        }
                                        println!();
                                        println!("   \x1b[33m⏳ Waiting for scan...\x1b[0m");
                                    }
                                }
                                "connected" => {
                                    let phone = event["phone"].as_str().unwrap_or("unknown");
                                    println!("\n   \x1b[32m✅ WhatsApp linked! Phone: {}\x1b[0m\n", phone);
                                    break;
                                }
                                "error" => {
                                    let msg = event["message"].as_str().unwrap_or("unknown");
                                    println!("   ❌ Bridge error: {}", msg);
                                    break;
                                }
                                _ => {}
                            }
                        }

                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
        }

        // ══════════════════════════════════════════════════════
        // Summary
        // ══════════════════════════════════════════════════════
        println!("\n\x1b[1m✅ Setup Complete!\x1b[0m\n");
        println!("   Config: {}", config_path.display());
        println!("   Skills: {}", skills_dir.display());
        println!();
        println!("   ┌──────────────────────────────────────────────┐");
        println!("   │  \x1b[1mConfiguration Summary\x1b[0m                        │");
        println!("   ├──────────────────────────────────────────────┤");
        println!("   │  Model:    {:<35}│", model);
        println!("   │  Autonomy: L{:<34}│", autonomy);

        let enabled_channels: Vec<&str> = channels.iter()
            .filter(|c| c.enabled)
            .map(|c| c.name.as_str())
            .collect();
        if !enabled_channels.is_empty() {
            let ch_str = enabled_channels.join(", ");
            println!("   │  Channels: {:<35}│", ch_str);
        } else {
            println!("   │  Channels: none                              │");
        }

        // WhatsApp needs QR linking
        let has_whatsapp = channels.iter().any(|c| c.channel_type == "whatsapp" && c.enabled);

        if !brave_key.is_empty() {
            println!("   │  Search:   Brave Search ✅                    │");
        }
        if mesh_enabled {
            println!("   │  Mesh:     enabled ({}){}│",
                mesh_capabilities.join(", "),
                " ".repeat(26_usize.saturating_sub(mesh_capabilities.join(", ").len())));
        }
        println!("   └──────────────────────────────────────────────┘");

        // Next steps
        println!("\n   \x1b[1mNext steps:\x1b[0m\n");
        let mut step = 1;

        if api_key_value.is_empty() && !env_var_name.is_empty() {
            println!("   {}. Add your API key:", step);
            println!("      claw config set {}_api_key YOUR_KEY", provider_prefix);
            step += 1;
        }

        if has_whatsapp && !link_now_channels.contains(&"whatsapp".to_string()) {
            println!("   {}. Link WhatsApp (scan QR code):", step);
            println!("      claw channels login whatsapp");
            step += 1;
        }

        let pending_channels: Vec<&str> = channels.iter()
            .filter(|c| !c.enabled && c.channel_type != "whatsapp")
            .map(|c| c.name.as_str())
            .collect();
        if !pending_channels.is_empty() {
            println!("   {}. Set up remaining channels:", step);
            for ch in &pending_channels {
                println!("      claw channels login {}", ch);
            }
            step += 1;
        }

        println!("   {}. Start the agent:   claw start", step);
        step += 1;
        println!("   {}. Chat interactively: claw chat", step);

        println!();
        println!("   Run 'claw doctor' to verify your configuration.");
        println!("   Run 'claw channels status' to see channel connection state.\n");

        Ok(())
    }

    fn cmd_version() -> claw_core::Result<()> {
        println!("🦞 Claw v{}", env!("CARGO_PKG_VERSION"));
        println!("   Rust edition: {}", "2024");
        println!("   Target: {}", std::env::consts::ARCH);
        println!("   OS: {}", std::env::consts::OS);
        #[cfg(debug_assertions)]
        println!("   Profile: debug");
        #[cfg(not(debug_assertions))]
        println!("   Profile: release");
        Ok(())
    }

    fn cmd_completions(shell: Shell) -> claw_core::Result<()> {
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "claw", &mut std::io::stdout());
        Ok(())
    }
}

/// Truncate a string to `max` characters, appending "..." if truncated.
fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.replace('\n', " ")
    } else {
        format!("{}...", &s[..max].replace('\n', " "))
    }
}