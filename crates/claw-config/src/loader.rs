use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

use crate::schema::ClawConfig;

/// Loads and optionally hot-reloads the Claw configuration.
pub struct ConfigLoader {
    config: Arc<RwLock<ClawConfig>>,
    config_path: PathBuf,
}

impl ConfigLoader {
    /// Resolve the config path: explicit path > CLAW_CONFIG env > ~/.claw/claw.toml
    pub fn resolve_path(explicit: Option<&Path>) -> PathBuf {
        if let Some(p) = explicit {
            return p.to_path_buf();
        }
        if let Ok(p) = std::env::var("CLAW_CONFIG") {
            return PathBuf::from(p);
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claw")
            .join("claw.toml")
    }

    /// Load the config from disk, falling back to defaults.
    pub fn load(path: Option<&Path>) -> claw_core::Result<Self> {
        let config_path = Self::resolve_path(path);
        let config = if config_path.exists() {
            info!(?config_path, "loading configuration");
            let raw = std::fs::read_to_string(&config_path)?;
            toml::from_str::<ClawConfig>(&raw).map_err(|e| {
                claw_core::ClawError::Config(format!(
                    "failed to parse {}: {}",
                    config_path.display(),
                    e
                ))
            })?
        } else {
            warn!(?config_path, "config file not found, using defaults");
            ClawConfig::default()
        };

        // Apply environment variable overrides
        let config = Self::apply_env_overrides(config);

        // Validate config — log warnings, fail on errors
        match config.validate() {
            Ok(warnings) => {
                for w in &warnings {
                    warn!("{}", w);
                }
            }
            Err(e) => {
                return Err(claw_core::ClawError::Config(e));
            }
        }

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
        })
    }

    /// Get a read snapshot of the current config.
    pub fn get(&self) -> ClawConfig {
        self.config.read().clone()
    }

    /// Get a shared reference for subscription.
    pub fn shared(&self) -> Arc<RwLock<ClawConfig>> {
        Arc::clone(&self.config)
    }

    /// Path being watched.
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// Apply env var overrides (CLAW_AGENT_MODEL, CLAW_AUTONOMY_LEVEL, etc.)
    fn apply_env_overrides(mut config: ClawConfig) -> ClawConfig {
        if let Ok(v) = std::env::var("CLAW_AGENT_MODEL") {
            config.agent.model = v;
        }
        if let Ok(v) = std::env::var("CLAW_AUTONOMY_LEVEL") {
            if let Ok(level) = v.parse::<u8>() {
                config.autonomy.level = level;
            }
        }
        if let Ok(v) = std::env::var("CLAW_SERVER_LISTEN") {
            config.server.listen = v;
        }
        if let Ok(v) = std::env::var("CLAW_LOG_LEVEL") {
            config.logging.level = v;
        }
        if let Ok(v) = std::env::var("CLAW_DAILY_BUDGET") {
            if let Ok(budget) = v.parse::<f64>() {
                config.autonomy.daily_budget_usd = budget;
            }
        }
        // API keys: env var fills in when config file doesn't have the key set.
        // This means config file takes priority, env is the fallback.
        if config.services.anthropic_api_key.is_none() {
            if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
                config.services.anthropic_api_key = Some(v);
            }
        }
        if config.services.openai_api_key.is_none() {
            if let Ok(v) = std::env::var("OPENAI_API_KEY") {
                config.services.openai_api_key = Some(v);
            }
        }
        if config.services.brave_api_key.is_none() {
            if let Ok(v) = std::env::var("BRAVE_API_KEY") {
                config.services.brave_api_key = Some(v);
            }
        }
        // 1Password service account token: config file takes priority, env var is fallback.
        if config.credentials.service_account_token.is_none() {
            if let Ok(v) = std::env::var("OP_SERVICE_ACCOUNT_TOKEN") {
                config.credentials.service_account_token = Some(v);
            }
        }
        config
    }

    /// Reload the config from disk.
    pub fn reload(&self) -> claw_core::Result<()> {
        if !self.config_path.exists() {
            return Err(claw_core::ClawError::Config(format!(
                "config file not found: {}",
                self.config_path.display()
            )));
        }
        let raw = std::fs::read_to_string(&self.config_path)?;
        let new_config = toml::from_str::<ClawConfig>(&raw).map_err(|e| {
            claw_core::ClawError::Config(format!(
                "failed to parse {}: {}",
                self.config_path.display(),
                e
            ))
        })?;
        let new_config = Self::apply_env_overrides(new_config);
        *self.config.write() = new_config;
        info!("configuration reloaded");
        Ok(())
    }

    /// Start a background file watcher that triggers `reload()` when the config file changes.
    /// Returns a handle to the watcher (must be kept alive for watching to continue).
    pub fn watch(&self) -> claw_core::Result<notify::RecommendedWatcher> {
        let config = Arc::clone(&self.config);
        let config_path = self.config_path.clone();

        info!(?config_path, "starting config file watcher");

        let path_for_event = config_path.clone();
        let mut watcher = notify::recommended_watcher(
            move |res: Result<NotifyEvent, notify::Error>| {
                match res {
                    Ok(event) => {
                        // Only react to modify/create events on our specific file
                        match event.kind {
                            EventKind::Modify(_) | EventKind::Create(_) => {
                                // Check that the event is for our file
                                let is_our_file = event.paths.iter().any(|p| {
                                    p.file_name() == path_for_event.file_name()
                                });
                                if !is_our_file {
                                    return;
                                }

                                info!("config file changed, reloading");
                                match std::fs::read_to_string(&path_for_event) {
                                    Ok(raw) => {
                                        match toml::from_str::<ClawConfig>(&raw) {
                                            Ok(new_config) => {
                                                let new_config = ConfigLoader::apply_env_overrides(new_config);
                                                *config.write() = new_config;
                                                info!("configuration hot-reloaded successfully");
                                            }
                                            Err(e) => {
                                                warn!(error = %e, "config file has errors, keeping current config");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "failed to read config file during hot-reload");
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "file watcher error");
                    }
                }
            }
        ).map_err(|e| {
            claw_core::ClawError::Config(format!("failed to create file watcher: {}", e))
        })?;

        // Watch the parent directory (some editors create temp files + rename)
        let watch_path = self.config_path.parent().unwrap_or(Path::new("."));
        watcher
            .watch(watch_path, RecursiveMode::NonRecursive)
            .map_err(|e| {
                claw_core::ClawError::Config(format!("failed to watch config directory: {}", e))
            })?;

        Ok(watcher)
    }
}
