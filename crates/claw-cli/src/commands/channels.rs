use std::path::PathBuf;

use super::ChannelAction;

pub(super) async fn cmd_channels(
    config: claw_config::ClawConfig,
    action: ChannelAction,
) -> claw_core::Result<()> {
    match action {
        ChannelAction::Status => {
            println!("\x1b[1m📡 Channel Status\x1b[0m\n");

            let cred_dir = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claw")
                .join("credentials");

            // Check WhatsApp
            let wa_linked = cred_dir.join("whatsapp").join("creds.json").exists();
            let wa_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "whatsapp");
            if wa_configured || wa_linked {
                let status = if wa_linked {
                    "\x1b[32m● linked\x1b[0m"
                } else {
                    "\x1b[31m○ not linked\x1b[0m"
                };
                let dm_policy = config
                    .channels
                    .iter()
                    .find(|(_, c)| c.channel_type == "whatsapp")
                    .map(|(_, c)| c.dm_policy.as_str())
                    .unwrap_or("pairing");
                println!("   📱 WhatsApp:  {status} (dm: {dm_policy})");
                if !wa_linked {
                    println!("      \x1b[90m→ Run: claw channels login whatsapp\x1b[0m");
                }
            }

            // Check Telegram
            let tg_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "telegram");
            if tg_configured {
                let has_token = config
                    .channels
                    .iter()
                    .find(|(_, c)| c.channel_type == "telegram")
                    .and_then(|(_, c)| c.settings.get("token"))
                    .is_some();
                let status = if has_token {
                    "\x1b[32m● configured\x1b[0m"
                } else {
                    "\x1b[33m○ no token\x1b[0m"
                };
                println!("   🤖 Telegram:  {status}");
            }

            // Check Discord
            let dc_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "discord");
            if dc_configured {
                let has_token = config
                    .channels
                    .iter()
                    .find(|(_, c)| c.channel_type == "discord")
                    .and_then(|(_, c)| c.settings.get("token"))
                    .is_some();
                let status = if has_token {
                    "\x1b[32m● configured\x1b[0m"
                } else {
                    "\x1b[33m○ no token\x1b[0m"
                };
                println!("   💬 Discord:   {status}");
            }

            // Check Signal
            let sig_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "signal");
            if sig_configured {
                let cli_ok = claw_channels::signal::SignalChannel::is_signal_cli_available();
                let status = if cli_ok {
                    "\x1b[32m● signal-cli found\x1b[0m"
                } else {
                    "\x1b[31m○ signal-cli not found\x1b[0m"
                };
                println!("   🔒 Signal:    {status}");
            }

            // Check Slack
            let sl_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "slack");
            if sl_configured {
                println!("   📎 Slack:     \x1b[32m● configured\x1b[0m");
            }

            // WebChat is always available
            let wc_configured = config
                .channels
                .iter()
                .any(|(_, c)| c.channel_type == "webchat");
            if wc_configured {
                println!(
                    "   🌐 WebChat:   \x1b[32m● enabled\x1b[0m (http://{})",
                    config.server.listen
                );
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
        ChannelAction::Login {
            channel,
            account: _,
            force,
        } => {
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
                            reason: format!("Failed to start bridge: {e}"),
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
                                match qrcode::QrCode::new(qr_data.as_bytes()) {
                                    Ok(code) => {
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
                                    Err(e) => {
                                        eprintln!(
                                            "   ⚠️  QR render failed: {e}. Raw data: {qr_data}"
                                        );
                                    }
                                }
                            }
                            "connected" => {
                                let phone = event["phone"].as_str().unwrap_or("unknown");
                                println!();
                                println!("   \x1b[32m✅ WhatsApp linked successfully!\x1b[0m");
                                println!("   Phone: {phone}");
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
                                eprintln!("   ❌ Bridge error: {msg}");
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
                            let mut doc =
                                content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {e}"))
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
                            let mut doc =
                                content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {e}"))
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
                            let mut doc =
                                content.parse::<toml_edit::DocumentMut>().map_err(|e| {
                                    claw_core::ClawError::Config(format!("Invalid TOML: {e}"))
                                })?;

                            doc["channels"]["slack"]["type"] = toml_edit::value("slack");
                            doc["channels"]["slack"]["bot_token"] =
                                toml_edit::value(&bot_token);

                            std::fs::write(&config_path, doc.to_string())?;
                            println!("\n   ✅ Slack bot configured!");
                        }
                    }
                }
                other => {
                    eprintln!("❌ Unknown channel: '{other}'");
                    eprintln!("   Supported: whatsapp, telegram, discord, signal, slack");
                }
            }
        }
        ChannelAction::Logout {
            channel,
            account: _,
        } => match channel.to_lowercase().as_str() {
            "whatsapp" | "wa" => {
                let wa = claw_channels::whatsapp::WhatsAppChannel::new("whatsapp".into(), None);
                wa.logout()?;
                println!(
                    "✅ WhatsApp session cleared. Re-link with: claw channels login whatsapp"
                );
            }
            other => {
                println!("Channel '{other}' logout: removing config entry.");
                println!("Edit claw.toml to fully remove the channel configuration.");
            }
        },
        ChannelAction::Pairing { channel } => match channel.to_lowercase().as_str() {
            "whatsapp" | "wa" => {
                let wa = claw_channels::whatsapp::WhatsAppChannel::new("whatsapp".into(), None);
                let requests = wa.load_pairing_requests();
                if requests.is_empty() {
                    println!("No pending WhatsApp pairing requests.");
                } else {
                    println!("\x1b[1mPending WhatsApp Pairing Requests:\x1b[0m\n");
                    for req in &requests {
                        println!("   Code: \x1b[1m{}\x1b[0m", req.code);
                        println!(
                            "   From: {} {}",
                            req.sender,
                            req.sender_name.as_deref().unwrap_or("")
                        );
                        println!("   Time: {}", req.created_at);
                        println!("   Expires: {}", req.expires_at);
                        println!();
                    }
                    println!("   Approve: claw channels approve whatsapp <CODE>");
                    println!("   Deny:    claw channels deny whatsapp <CODE>");
                }
            }
            other => {
                println!("Pairing for '{other}' — checking credential store...");
                let cred_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".claw")
                    .join("credentials")
                    .join(other);
                let pairing_file = cred_dir.join("pairing.json");
                if pairing_file.exists() {
                    let data = std::fs::read_to_string(&pairing_file)?;
                    println!("{data}");
                } else {
                    println!("No pending pairing requests for '{other}'.");
                }
            }
        },
        ChannelAction::Approve { channel, code } => match channel.to_lowercase().as_str() {
            "whatsapp" | "wa" => {
                let wa = claw_channels::whatsapp::WhatsAppChannel::new("whatsapp".into(), None);
                match wa.approve_pairing(&code) {
                    Ok(sender) => {
                        println!("✅ Approved pairing for {sender} on WhatsApp");
                    }
                    Err(e) => {
                        eprintln!("❌ {e}");
                    }
                }
            }
            other => {
                println!(
                    "Pairing approval for '{other}' with code '{code}' — not yet implemented."
                );
            }
        },
        ChannelAction::Deny { channel, code } => match channel.to_lowercase().as_str() {
            "whatsapp" | "wa" => {
                let wa = claw_channels::whatsapp::WhatsAppChannel::new("whatsapp".into(), None);
                wa.deny_pairing(&code)?;
                println!("❌ Denied pairing code {code}");
            }
            _ => {
                println!("Denied pairing code {code} for {channel}");
            }
        },
    }
    Ok(())
}
