use std::sync::Arc;
use tracing::error;

use claw_config::ConfigLoader;
use claw_runtime::AgentRuntime;

pub(super) async fn cmd_start(
    config: claw_config::ClawConfig,
    no_server: bool,
    config_loader: ConfigLoader,
) -> claw_core::Result<()> {
    println!("🦞 Claw v{}", env!("CARGO_PKG_VERSION"));
    println!("   Model: {}", config.agent.model);
    println!("   Autonomy: L{}", config.autonomy.level);
    println!();

    // Background update check — non-blocking, best-effort
    tokio::spawn(async {
        match super::update::check_for_update().await {
            Some((current, latest)) => {
                println!("   📦 Update available: v{current} → v{latest}  (run `claw update`)");
            }
            None => {} // up to date or network error
        }
    });

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
                eprintln!("   Your model is '{model}'. Set your key:");
                eprintln!("   In claw.toml:  [services]");
                eprintln!("                  anthropic_api_key = \"sk-ant-...\"");
                eprintln!("   Or env var:    export ANTHROPIC_API_KEY=sk-ant-...");
            } else if model.starts_with("openai/") {
                eprintln!("   Your model is '{model}'. Set your key:");
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
                if let Some(token) = channel_config
                    .settings
                    .get("token")
                    .and_then(|v| v.as_str())
                {
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
                let dm_policy_str = channel_config
                    .settings
                    .get("dm_policy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pairing");
                let dm_policy = match dm_policy_str {
                    "allowlist" => claw_channels::whatsapp::DmPolicy::Allowlist,
                    "open" => claw_channels::whatsapp::DmPolicy::Open,
                    "disabled" => claw_channels::whatsapp::DmPolicy::Disabled,
                    _ => claw_channels::whatsapp::DmPolicy::Pairing,
                };
                let allow_from: Vec<String> = channel_config
                    .settings
                    .get("allow_from")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let channel = claw_channels::whatsapp::WhatsAppChannel::new(id.clone(), None)
                    .with_dm_policy(dm_policy)
                    .with_allow_from(allow_from);
                runtime.add_channel(Box::new(channel));
                println!("   📱 WhatsApp: enabled (dm_policy={dm_policy_str})");
                println!("      Link your phone: claw channels login whatsapp");
            }
            "discord" => {
                if let Some(token) = channel_config
                    .settings
                    .get("token")
                    .and_then(|v| v.as_str())
                {
                    let channel =
                        claw_channels::discord::DiscordChannel::new(id.clone(), token.to_string());
                    runtime.add_channel(Box::new(channel));
                } else {
                    tracing::warn!("discord channel '{}' has no token configured", id);
                }
            }
            "slack" => {
                if let Some(token) = channel_config
                    .settings
                    .get("token")
                    .and_then(|v| v.as_str())
                {
                    let app_token = channel_config
                        .settings
                        .get("app_token")
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
                let phone = channel_config
                    .settings
                    .get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !phone.is_empty() {
                    let channel =
                        claw_channels::signal::SignalChannel::new(id.clone(), phone.to_string());
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

        // Resolve local skills/plugins directories so the Hub page can show them
        let config_base = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".claw");
        let plugin_dir = config.plugins.plugin_dir.clone();
        let skills_dir = if plugin_dir.is_absolute() {
            plugin_dir
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("skills")
        } else {
            config_base.join("skills")
        };

        tokio::spawn(async move {
            if let Err(e) =
                claw_server::start_server(server_config, hub_url, skills_dir, plugin_dir).await
            {
                error!(error = %e, "API server failed");
            }
        });
    }

    // Run the agent runtime (blocks until shutdown)
    runtime.run().await
}
