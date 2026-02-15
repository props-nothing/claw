use std::path::Path;
use tracing::info;

/// Plugin registry client — downloads plugins from ClawHub.
pub struct PluginRegistry {
    client: reqwest::Client,
    registry_url: String,
}

impl PluginRegistry {
    pub fn new(registry_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            registry_url: registry_url.to_string(),
        }
    }

    /// Search for plugins by query.
    pub async fn search(&self, query: &str) -> claw_core::Result<Vec<RegistryEntry>> {
        let url = format!("{}/api/v1/plugins?q={}", self.registry_url, query);
        let resp =
            self.client
                .get(&url)
                .send()
                .await
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: "registry".into(),
                    reason: e.to_string(),
                })?;

        let entries: Vec<RegistryEntry> =
            resp.json()
                .await
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: "registry".into(),
                    reason: e.to_string(),
                })?;

        Ok(entries)
    }

    /// Download and install a plugin to the given directory.
    pub async fn install(
        &self,
        name: &str,
        version: Option<&str>,
        dest_dir: &Path,
    ) -> claw_core::Result<()> {
        let version_part = version.unwrap_or("latest");
        let url = format!(
            "{}/api/v1/plugins/{}/{}",
            self.registry_url, name, version_part
        );

        info!(plugin = name, version = version_part, "downloading plugin");

        let resp =
            self.client
                .get(&url)
                .send()
                .await
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: name.to_string(),
                    reason: e.to_string(),
                })?;

        if !resp.status().is_success() {
            return Err(claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("registry returned HTTP {}", resp.status()),
            });
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: e.to_string(),
            })?;

        // Extract the tarball/zip to dest_dir/name/
        let plugin_dir = dest_dir.join(name);
        std::fs::create_dir_all(&plugin_dir)?;

        // For now, write as raw .wasm — production would handle tarballs
        let wasm_path = plugin_dir.join(format!("{name}.wasm"));
        std::fs::write(&wasm_path, &bytes)?;

        info!(plugin = name, path = ?wasm_path, "plugin installed");
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub downloads: u64,
    pub checksum: String,
}
