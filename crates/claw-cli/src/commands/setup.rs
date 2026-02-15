use std::path::PathBuf;

/// Initialize a new claw configuration with sensible defaults.
pub(super) fn cmd_init(local: bool) -> claw_core::Result<()> {
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

    let _default_config = include_str!("../../../claw-config/src/schema.rs");
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

/// Interactive setup wizard for claw configuration.
pub(super) fn cmd_setup(local: bool, reset: bool, section: Option<String>) -> claw_core::Result<()> {
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
            .items([
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
                    let sections = &[
                        "model", "channels", "autonomy", "services", "mesh", "server",
                    ];
                    let idx = Select::with_theme(&theme)
                        .with_prompt("Which section to configure?")
                        .items(sections)
                        .default(1) // default to channels
                        .interact()
                        .unwrap_or(1);
                    println!(
                        "\n   Tip: You can also run: claw setup --section {}",
                        sections[idx]
                    );
                    // TODO: edit individual section in existing config
                    println!(
                        "   Section editing for '{}' coming soon. For now use: claw config set KEY VALUE",
                        sections[idx]
                    );
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

        let providers = &[
            "Anthropic (Claude) — recommended",
            "OpenAI (GPT-4o, o3, etc.)",
            "Ollama (local, free, private)",
        ];
        let provider_idx = Select::with_theme(&theme)
            .with_prompt("Which LLM provider?")
            .items(providers)
            .default(0)
            .interact()
            .unwrap_or(0);

        let info = match provider_idx {
            0 => (
                "anthropic",
                "anthropic/claude-sonnet-4-20250514",
                "ANTHROPIC_API_KEY",
                "sk-ant-...",
            ),
            1 => ("openai", "openai/gpt-4o", "OPENAI_API_KEY", "sk-..."),
            2 => ("ollama", "ollama/llama3", "", ""),
            _ => (
                "anthropic",
                "anthropic/claude-sonnet-4-20250514",
                "ANTHROPIC_API_KEY",
                "sk-ant-...",
            ),
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
                println!("   ✅ {env_var_name} found in environment");
            } else {
                println!("\n   You need an API key from your provider.");
                if provider_prefix == "anthropic" {
                    println!("   Get one at: https://console.anthropic.com/settings/keys");
                } else if provider_prefix == "openai" {
                    println!("   Get one at: https://platform.openai.com/api-keys");
                }
                let key: String = Input::with_theme(&theme)
                    .with_prompt(format!(
                        "{provider_prefix} API key (or press Enter to skip)"
                    ))
                    .allow_empty(true)
                    .interact_text()
                    .unwrap_or_default();
                if !key.is_empty() {
                    api_key_value = key;
                    println!("   ✅ API key saved to config");
                } else {
                    println!(
                        "   ⚠️  Skipped — set later: claw config set {provider_prefix}_api_key YOUR_KEY"
                    );
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
        let channel_types = &[
            "whatsapp", "telegram", "discord", "slack", "signal", "webchat",
        ];

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
                                .with_prompt(
                                    "Allowed phone numbers (comma-separated, e.g. +1234567890)",
                                )
                                .interact_text()
                                .unwrap_or_default();
                            setup.allow_from = nums
                                .split(',')
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
                    println!(
                        "   2. Add Bot Token Scopes: chat:write, channels:history, im:history"
                    );
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
                        println!(
                            "   ⚠️  signal-cli not found. Install it before using Signal."
                        );
                    }

                    let phone: String = Input::with_theme(&theme)
                        .with_prompt("Signal phone number (e.g. +1234567890)")
                        .allow_empty(true)
                        .interact_text()
                        .unwrap_or_default();

                    if !phone.is_empty() {
                        setup.token = phone;
                        println!(
                            "   📌 You'll need to verify this number with signal-cli register/verify."
                        );
                    } else {
                        println!("   ⚠️  Skipped — set up later: claw channels login signal");
                        setup.enabled = false;
                    }
                }
                "webchat" => {
                    println!("\n   \x1b[1m🌐 Web Chat\x1b[0m");
                    println!(
                        "   The built-in web UI will be available when you run 'claw start'."
                    );
                    println!("   Access it at http://localhost:3700 in your browser.\n");
                }
                _ => {}
            }

            channels.push(setup);
        }

        if selected.is_empty() {
            println!(
                "\n   No channels selected. You can add them later: claw channels login <channel>"
            );
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
            .with_prompt(
                "Enable web search? (Brave Search — free at https://api.search.brave.com/)",
            )
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
            mesh_capabilities = caps
                .split(',')
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
    config.push_str(&format!("model = \"{model}\"\n"));
    if provider_prefix == "anthropic" {
        config.push_str("# thinking_level = \"medium\"\n");
    }
    config.push_str("# temperature = 0.7\n");
    config.push_str("# max_tokens = 8192\n\n");

    // Autonomy section
    config.push_str("[autonomy]\n");
    config.push_str(&format!("level = {autonomy}\n"));
    config.push_str(&format!("daily_budget_usd = {budget}\n"));
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
    config.push_str(&format!("listen = \"{listen}\"\n"));
    config.push_str("web_ui = true\n\n");

    // Services section
    config.push_str("[services]\n");
    if provider_prefix == "anthropic" {
        if !api_key_value.is_empty() {
            config.push_str(&format!("anthropic_api_key = \"{api_key_value}\"\n"));
        } else {
            config.push_str(
                "# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n",
            );
        }
        config.push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
    } else if provider_prefix == "openai" {
        config.push_str("# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n");
        if !api_key_value.is_empty() {
            config.push_str(&format!("openai_api_key = \"{api_key_value}\"\n"));
        } else {
            config
                .push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
        }
    } else {
        config.push_str("# anthropic_api_key = \"sk-ant-...\"   # or env: ANTHROPIC_API_KEY\n");
        config.push_str("# openai_api_key = \"sk-...\"          # or env: OPENAI_API_KEY\n");
    }
    if !brave_key.is_empty() {
        config.push_str(&format!("brave_api_key = \"{brave_key}\"\n"));
    } else {
        config.push_str(
            "# brave_api_key = \"\"    # Get one free: https://api.search.brave.com/\n",
        );
    }
    config.push_str(
        "# hub_url = \"\"          # Skills Hub URL — run 'claw hub serve' to host one\n",
    );
    config.push('\n');

    // ── Channels section ─────────────────────────────────
    config.push_str("# ── Channels ────────────────────────────────────────────────────\n");
    config.push_str("# Connect your agent to messaging apps.\n");
    config.push_str("# Manage channels:  claw channels status\n");
    config.push_str("# Login:            claw channels login whatsapp\n");
    config.push_str("# DM policies:      pairing | allowlist | open | disabled\n\n");

    for ch in &channels {
        if !ch.enabled && ch.token.is_empty() {
            config.push_str(&format!(
                "# [channels.{}]    # run: claw channels login {}\n",
                ch.name, ch.name
            ));
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
                let nums: Vec<String> =
                    ch.allow_from.iter().map(|n| format!("\"{n}\"")).collect();
                config.push_str(&format!("allow_from = [{}]\n", nums.join(", ")));
            }
        }

        config.push('\n');
    }

    // Mesh section
    config.push_str("# ── Mesh Network ────────────────────────────────────────────────\n\n");
    config.push_str("[mesh]\n");
    config.push_str(&format!("enabled = {mesh_enabled}\n"));
    config.push_str(&format!("listen = \"{mesh_listen}\"\n"));
    config.push_str("mdns = true\n");
    let caps_toml: Vec<String> = mesh_capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect();
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
        (
            "plesk-server",
            include_str!("../../../../skills/plesk-server/SKILL.md"),
        ),
        ("github", include_str!("../../../../skills/github/SKILL.md")),
        ("docker", include_str!("../../../../skills/docker/SKILL.md")),
        (
            "server-management",
            include_str!("../../../../skills/server-management/SKILL.md"),
        ),
        ("coding", include_str!("../../../../skills/coding/SKILL.md")),
        (
            "web-research",
            include_str!("../../../../skills/web-research/SKILL.md"),
        ),
        (
            "system-admin",
            include_str!("../../../../skills/system-admin/SKILL.md"),
        ),
        (
            "1password",
            include_str!("../../../../skills/1password/SKILL.md"),
        ),
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
                        println!("   ❌ Bridge install failed: {e}");
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
                        let line = match line {
                            Ok(l) => l,
                            Err(_) => break,
                        };
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let event: serde_json::Value = match serde_json::from_str(trimmed) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        match event["type"].as_str().unwrap_or("") {
                            "qr" => {
                                let qr_data = event["data"].as_str().unwrap_or("");
                                if let Ok(code) = qrcode::QrCode::new(qr_data.as_bytes()) {
                                    let width = code.width();
                                    let colors: Vec<bool> = code
                                        .into_colors()
                                        .into_iter()
                                        .map(|c| c == qrcode::Color::Dark)
                                        .collect();
                                    let quiet = 1i32;
                                    let at = |x: i32, y: i32| -> bool {
                                        if x < 0
                                            || y < 0
                                            || x >= width as i32
                                            || y >= width as i32
                                        {
                                            false
                                        } else {
                                            colors[(y as usize) * width + (x as usize)]
                                        }
                                    };
                                    let total = width as i32 + quiet * 2;

                                    println!(
                                        "   \x1b[1m📱 Scan this QR code with WhatsApp:\x1b[0m\n"
                                    );
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
                                        println!("{row}");
                                        y += 2;
                                    }
                                    println!();
                                    println!("   \x1b[33m⏳ Waiting for scan...\x1b[0m");
                                }
                            }
                            "connected" => {
                                let phone = event["phone"].as_str().unwrap_or("unknown");
                                println!(
                                    "\n   \x1b[32m✅ WhatsApp linked! Phone: {phone}\x1b[0m\n"
                                );
                                break;
                            }
                            "error" => {
                                let msg = event["message"].as_str().unwrap_or("unknown");
                                println!("   ❌ Bridge error: {msg}");
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
    println!("   │  Model:    {model:<35}│");
    println!("   │  Autonomy: L{autonomy:<34}│");

    let enabled_channels: Vec<&str> = channels
        .iter()
        .filter(|c| c.enabled)
        .map(|c| c.name.as_str())
        .collect();
    if !enabled_channels.is_empty() {
        let ch_str = enabled_channels.join(", ");
        println!("   │  Channels: {ch_str:<35}│");
    } else {
        println!("   │  Channels: none                              │");
    }

    // WhatsApp needs QR linking
    let has_whatsapp = channels
        .iter()
        .any(|c| c.channel_type == "whatsapp" && c.enabled);

    if !brave_key.is_empty() {
        println!("   │  Search:   Brave Search ✅                    │");
    }
    if mesh_enabled {
        println!(
            "   │  Mesh:     enabled ({}){}│",
            mesh_capabilities.join(", "),
            " ".repeat(26_usize.saturating_sub(mesh_capabilities.join(", ").len()))
        );
    }
    println!("   └──────────────────────────────────────────────┘");

    // Next steps
    println!("\n   \x1b[1mNext steps:\x1b[0m\n");
    let mut step = 1;

    if api_key_value.is_empty() && !env_var_name.is_empty() {
        println!("   {step}. Add your API key:");
        println!("      claw config set {provider_prefix}_api_key YOUR_KEY");
        step += 1;
    }

    if has_whatsapp && !link_now_channels.contains(&"whatsapp".to_string()) {
        println!("   {step}. Link WhatsApp (scan QR code):");
        println!("      claw channels login whatsapp");
        step += 1;
    }

    let pending_channels: Vec<&str> = channels
        .iter()
        .filter(|c| !c.enabled && c.channel_type != "whatsapp")
        .map(|c| c.name.as_str())
        .collect();
    if !pending_channels.is_empty() {
        println!("   {step}. Set up remaining channels:");
        for ch in &pending_channels {
            println!("      claw channels login {ch}");
        }
        step += 1;
    }

    println!("   {step}. Start the agent:   claw start");
    step += 1;
    println!("   {step}. Chat interactively: claw chat");

    println!();
    println!("   Run 'claw doctor' to verify your configuration.");
    println!("   Run 'claw channels status' to see channel connection state.\n");

    Ok(())
}
