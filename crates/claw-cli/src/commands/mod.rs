use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::path::PathBuf;

use claw_config::ConfigLoader;

mod channels;
mod chat;
mod mesh;
mod plugins;
mod setup;
mod skills;
mod start;

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
    Install {
        name: String,
        #[arg(name = "ver")]
        version: Option<String>,
    },
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
        #[arg(short = 'L', long, default_value = "0.0.0.0:3800")]
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
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
                )
                .json()
                .with_target(true)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
                )
                .with_target(false)
                .init();
        }

        match self.command {
            Commands::Start { no_server } => {
                start::cmd_start(config, no_server, config_loader).await
            }
            Commands::Chat { session } => chat::cmd_chat(config, session).await,
            Commands::Status => Self::cmd_status(config).await,
            Commands::Version => Self::cmd_version(),
            Commands::Config { json } => Self::cmd_config(config, json),
            Commands::Plugin { action } => plugins::cmd_plugin(config, action).await,
            Commands::Logs {
                limit,
                event_type,
                json,
            } => Self::cmd_logs(config, limit, event_type, json).await,
            Commands::Set { key, value } => {
                Self::cmd_config_set(Some(config_loader.path().to_path_buf()), key, value)
            }
            Commands::Doctor => Self::cmd_doctor(config),
            Commands::Init { local } => setup::cmd_init(local),
            Commands::Setup {
                local,
                reset,
                section,
            } => setup::cmd_setup(local, reset, section),
            Commands::Completions { shell } => Self::cmd_completions(shell),
            Commands::Skill { action } => {
                skills::cmd_skill(config, action, config_loader.path()).await
            }
            Commands::Hub { action } => skills::cmd_hub(action).await,
            Commands::Mesh { action } => mesh::cmd_mesh(config, action).await,
            Commands::Channels { action } => channels::cmd_channels(config, action).await,
        }
    }

    async fn cmd_status(config: claw_config::ClawConfig) -> claw_core::Result<()> {
        let listen = &config.server.listen;
        println!("Checking status at http://{listen}...");

        let client = reqwest::Client::builder()
            .tcp_keepalive(None)
            .build()
            .unwrap_or_default();
        match client
            .get(format!("http://{listen}/api/v1/status"))
            .send()
            .await
        {
            Ok(resp) => {
                let data: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;
                println!("{}", serde_json::to_string_pretty(&data).unwrap());
            }
            Err(_) => {
                println!("❌ Agent is not running at {listen}");
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

    async fn cmd_logs(
        config: claw_config::ClawConfig,
        limit: usize,
        event_type: Option<String>,
        json: bool,
    ) -> claw_core::Result<()> {
        let listen = &config.server.listen;
        let client = reqwest::Client::builder()
            .tcp_keepalive(None)
            .build()
            .unwrap_or_default();
        let url = format!("http://{listen}/api/v1/audit?limit={limit}");

        let mut req = client.get(&url);
        if let Some(ref key) = config.server.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req.send().await.map_err(|e| {
            claw_core::ClawError::Agent(format!(
                "Cannot reach agent at {listen} — is it running? ({e})"
            ))
        })?;

        if !resp.status().is_success() {
            return Err(claw_core::ClawError::Agent(format!(
                "Server returned {}",
                resp.status()
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

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
            entries
                .iter()
                .filter(|e| {
                    e["event_type"]
                        .as_str()
                        .map(|t| t.contains(et.as_str()))
                        .unwrap_or(false)
                })
                .collect()
        } else {
            entries.iter().collect()
        };

        if filtered.is_empty() {
            println!(
                "No audit log entries{}",
                event_type
                    .as_ref()
                    .map(|t| format!(" matching '{t}'"))
                    .unwrap_or_default()
            );
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
                t if t.contains("approval") => "\x1b[33m",                          // yellow
                t if t.contains("tool") => "\x1b[36m",                              // cyan
                t if t.contains("budget") => "\x1b[35m",                            // magenta
                _ => "\x1b[37m",                                                    // default
            };

            println!("\x1b[90m{ts}\x1b[0m  {color}{etype}\x1b[0m  {action}");
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
            None => println!("✅ {key} = {value} (new)"),
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
                println!("{e}");
                return Ok(());
            }
        };

        let mut warn_count = 0;
        let mut info_count = 0;

        for w in &warnings {
            println!("  {w}");
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
            println!(
                "  💡 autonomy.tool_denylist: no tools on denylist — consider blocking dangerous tools"
            );
            info_count += 1;
        } else {
            extra_ok += 1;
        }

        // Check if API key is set (for any bind address)
        if config.server.api_key.is_some() {
            extra_ok += 1;
        }

        println!();
        let ok_total = extra_ok
            + (if warnings.is_empty() {
                5
            } else {
                5 - warn_count - info_count
            });
        println!(
            "  ✅ {ok_total} checks passed, ⚠️  {warn_count} warnings, 💡 {info_count} suggestions"
        );

        Ok(())
    }

    fn cmd_version() -> claw_core::Result<()> {
        println!("🦞 Claw v{}", env!("CARGO_PKG_VERSION"));
        println!("   Rust edition: 2024");
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
