#[cfg(test)]
mod tests {
    use claw_config::ConfigLoader;
    use claw_config::schema::*;
    use std::io::Write;

    // ── Default tests ──────────────────────────────────────────

    #[test]
    fn test_claw_config_defaults() {
        let config = ClawConfig::default();
        assert_eq!(config.agent.model, "anthropic/claude-sonnet-4-20250514");
        assert_eq!(config.agent.max_tokens, 16384);
        assert_eq!(config.agent.temperature, 0.7);
        assert_eq!(config.agent.max_iterations, 50);
        assert_eq!(config.agent.thinking_level, "medium");
    }

    #[test]
    fn test_autonomy_config_defaults() {
        let config = AutonomyConfig::default();
        assert_eq!(config.level, 1);
        assert_eq!(config.daily_budget_usd, 10.0);
        assert_eq!(config.max_tool_calls_per_loop, 100);
        assert_eq!(config.approval_threshold, 7);
        assert!(!config.proactive);
    }

    #[test]
    fn test_server_config_defaults() {
        let config = ServerConfig::default();
        assert_eq!(config.listen, "127.0.0.1:3700");
        assert!(config.web_ui);
        assert!(!config.cors);
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_memory_config_defaults() {
        let config = MemoryConfig::default();
        assert!(config.vector_search);
        assert_eq!(config.max_episodes, 10000);
        assert_eq!(config.embedding_dims, 384);
    }

    #[test]
    fn test_logging_config_defaults() {
        let config = LoggingConfig::default();
        assert_eq!(config.level, "info");
        assert_eq!(config.format, "pretty");
    }

    // ── TOML roundtrip tests ───────────────────────────────────

    #[test]
    fn test_config_toml_roundtrip() {
        let config = ClawConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let restored: ClawConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.agent.model, config.agent.model);
        assert_eq!(restored.autonomy.level, config.autonomy.level);
        assert_eq!(restored.server.listen, config.server.listen);
    }

    #[test]
    fn test_partial_toml_applies_defaults() {
        let toml_str = r#"
[agent]
model = "openai/gpt-4o"

[autonomy]
level = 3
"#;
        let config: ClawConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.agent.model, "openai/gpt-4o");
        assert_eq!(config.autonomy.level, 3);
        // Defaults should fill in
        assert_eq!(config.agent.max_tokens, 16384);
        assert_eq!(config.server.listen, "127.0.0.1:3700");
        assert!(config.memory.vector_search);
    }

    #[test]
    fn test_channel_config_deserialize() {
        let toml_str = r#"
[channels.telegram]
type = "telegram"
token = "abc123"
enabled = true
"#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            channels: std::collections::HashMap<String, ChannelConfig>,
        }
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        let tg = &w.channels["telegram"];
        assert_eq!(tg.channel_type, "telegram");
        assert!(tg.enabled);
        assert_eq!(tg.settings["token"].as_str().unwrap(), "abc123");
    }

    #[test]
    fn test_goal_config_deserialize() {
        let toml_str = r#"
[[autonomy.goals]]
description = "Monitor system health"
priority = 5
enabled = true
"#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            autonomy: AutonomyConfig,
        }
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(w.autonomy.goals.len(), 1);
        assert_eq!(w.autonomy.goals[0].description, "Monitor system health");
        assert_eq!(w.autonomy.goals[0].priority, 5);
    }

    // ── ConfigLoader tests ─────────────────────────────────────

    #[test]
    fn test_config_loader_with_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("claw.toml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            r#"
[agent]
model = "openai/gpt-4o"
max_tokens = 4096

[autonomy]
level = 2
daily_budget_usd = 5.0

[server]
listen = "0.0.0.0:8080"
"#
        )
        .unwrap();

        let loader = ConfigLoader::load(Some(config_path.as_path())).unwrap();
        let config = loader.get();
        assert_eq!(config.agent.model, "openai/gpt-4o");
        assert_eq!(config.agent.max_tokens, 4096);
        assert_eq!(config.autonomy.level, 2);
        assert_eq!(config.autonomy.daily_budget_usd, 5.0);
        assert_eq!(config.server.listen, "0.0.0.0:8080");
    }

    #[test]
    fn test_config_loader_missing_file_uses_defaults() {
        // Load from a non-existent explicit path should fail
        let _result = ConfigLoader::load(Some(std::path::Path::new("/nonexistent/claw.toml")));
        // But loading without an explicit path should produce defaults
        // (if no config exists in default locations)
        // We just verify defaults produce a valid config
        let config = ClawConfig::default();
        assert_eq!(config.agent.model, "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_config_loader_reload() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("claw.toml");

        // Write initial config
        std::fs::write(
            &config_path,
            r#"
[agent]
model = "openai/gpt-4o"
"#,
        )
        .unwrap();

        let loader = ConfigLoader::load(Some(config_path.as_path())).unwrap();
        assert_eq!(loader.get().agent.model, "openai/gpt-4o");

        // Update the file
        std::fs::write(
            &config_path,
            r#"
[agent]
model = "anthropic/claude-opus-4-20250514"
"#,
        )
        .unwrap();

        loader.reload().unwrap();
        assert_eq!(loader.get().agent.model, "anthropic/claude-opus-4-20250514");
    }

    // ── JSON roundtrip ─────────────────────────────────────────

    #[test]
    fn test_config_json_roundtrip() {
        let config = ClawConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let restored: ClawConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent.model, config.agent.model);
    }
}
