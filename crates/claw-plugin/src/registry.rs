use std::path::Path;
use tracing::info;

/// Plugin registry client — downloads plugins from the Claw Hub.
///
/// The registry URL should point to the same hub as `services.hub_url`.
/// It uses the `/api/v1/hub/plugins/*` endpoints served by `claw hub serve`.
pub struct PluginRegistry {
    client: reqwest::Client,
    registry_url: String,
}

impl PluginRegistry {
    pub fn new(registry_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            registry_url: registry_url.trim_end_matches('/').to_string(),
        }
    }

    /// Search for plugins by query.
    pub async fn search(&self, query: &str) -> claw_core::Result<Vec<RegistryEntry>> {
        let url = format!(
            "{}/api/v1/hub/plugins/search?q={}",
            self.registry_url, query
        );
        let resp =
            self.client
                .get(&url)
                .send()
                .await
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: "registry".into(),
                    reason: format!("cannot reach hub at {}: {e}", self.registry_url),
                })?;

        if !resp.status().is_success() {
            return Err(claw_core::ClawError::Plugin {
                plugin: "registry".into(),
                reason: format!("hub returned HTTP {}", resp.status()),
            });
        }

        // The hub returns { "plugins": [...], "query": "..." }
        let data: serde_json::Value =
            resp.json()
                .await
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: "registry".into(),
                    reason: e.to_string(),
                })?;

        let entries: Vec<RegistryEntry> = data
            .get("plugins")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(entries)
    }

    /// Download and install a plugin to the given directory.
    ///
    /// Downloads the WASM binary + reconstructs `plugin.toml` from hub metadata.
    pub async fn install(
        &self,
        name: &str,
        version: Option<&str>,
        dest_dir: &Path,
    ) -> claw_core::Result<()> {
        let version_part = version.unwrap_or("latest");

        // First get plugin metadata (to write plugin.toml locally)
        let meta_url = format!("{}/api/v1/hub/plugins/{}", self.registry_url, name);
        info!(plugin = name, version = version_part, url = %meta_url, "downloading plugin from hub");

        let meta_resp = self
            .client
            .get(&meta_url)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("cannot reach hub: {e}"),
            })?;

        if !meta_resp.status().is_success() {
            return Err(claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("plugin '{}' not found on hub (HTTP {})", name, meta_resp.status()),
            });
        }

        // Download the WASM binary
        let wasm_url = format!(
            "{}/api/v1/hub/plugins/{}/{}",
            self.registry_url, name, version_part
        );

        let wasm_resp = self
            .client
            .get(&wasm_url)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("failed to download WASM: {e}"),
            })?;

        if !wasm_resp.status().is_success() {
            return Err(claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("hub returned HTTP {} for WASM download", wasm_resp.status()),
            });
        }

        let wasm_bytes = wasm_resp
            .bytes()
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: e.to_string(),
            })?;

        // Get the metadata JSON to reconstruct a local plugin.toml
        let meta: serde_json::Value = meta_resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("invalid metadata JSON: {e}"),
            })?;

        // Write to dest_dir/name/
        let plugin_dir = dest_dir.join(name);
        std::fs::create_dir_all(&plugin_dir)?;

        // Write WASM binary
        let wasm_path = plugin_dir.join(format!("{name}.wasm"));
        std::fs::write(&wasm_path, &wasm_bytes)?;

        // Reconstruct a minimal plugin.toml from hub metadata
        let checksum = blake3::hash(&wasm_bytes).to_hex().to_string();
        let version_str = meta["version"].as_str().unwrap_or("0.1.0");
        let description = meta["description"].as_str().unwrap_or("");
        let license = meta["license"].as_str().unwrap_or("");
        let authors: Vec<String> = meta
            .get("authors")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut manifest = format!(
            "[plugin]\nname = \"{name}\"\nversion = \"{version_str}\"\ndescription = \"{description}\"\n"
        );
        if !authors.is_empty() {
            manifest.push_str(&format!(
                "authors = [{}]\n",
                authors
                    .iter()
                    .map(|a| format!("\"{a}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !license.is_empty() {
            manifest.push_str(&format!("license = \"{license}\"\n"));
        }
        manifest.push_str(&format!("checksum = \"{checksum}\"\n"));

        // Add tools from hub metadata
        if let Some(tools) = meta.get("tools").and_then(|v| v.as_array()) {
            for tool in tools {
                let tool_name = tool["name"].as_str().unwrap_or("unknown");
                let tool_desc = tool["description"].as_str().unwrap_or("");
                let risk = tool["risk_level"].as_u64().unwrap_or(0);
                let mutating = tool["is_mutating"].as_bool().unwrap_or(false);
                manifest.push_str(&format!(
                    "\n[[tools]]\nname = \"{tool_name}\"\ndescription = \"{tool_desc}\"\nrisk_level = {risk}\nis_mutating = {mutating}\nparameters = {{}}\n"
                ));
            }
        }

        std::fs::write(plugin_dir.join("plugin.toml"), &manifest)?;

        info!(plugin = name, version = version_str, path = ?wasm_path, size = wasm_bytes.len(), "plugin installed from hub");
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub downloads: u64,
    #[serde(default)]
    pub checksum: String,
    #[serde(default)]
    pub wasm_size: u64,
    #[serde(default)]
    pub tools: Vec<RegistryToolEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistryToolEntry {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub risk_level: u8,
    #[serde(default)]
    pub is_mutating: bool,
}
