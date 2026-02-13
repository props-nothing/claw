use serde::{Deserialize, Serialize};
use semver::Version;

/// Plugin manifest — loaded from `plugin.toml` alongside the `.wasm` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// BLAKE3 hash of the .wasm file for integrity verification.
    #[serde(default)]
    pub checksum: Option<String>,
}

/// Capabilities the plugin requests from the host.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginCapabilities {
    /// Allowed network URLs (glob patterns).
    #[serde(default)]
    pub network: Vec<String>,
    /// Allowed filesystem paths (glob patterns).
    #[serde(default)]
    pub filesystem: Vec<String>,
    /// Whether the plugin needs shell access.
    #[serde(default)]
    pub shell: bool,
    /// Custom host functions the plugin requires.
    #[serde(default)]
    pub host_functions: Vec<String>,
}

/// A tool defined by the plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub risk_level: u8,
    #[serde(default)]
    pub is_mutating: bool,
}

impl PluginManifest {
    /// Parse from TOML string.
    pub fn from_toml(s: &str) -> claw_core::Result<Self> {
        toml::from_str(s).map_err(|e| {
            claw_core::ClawError::Plugin {
                plugin: "unknown".into(),
                reason: format!("failed to parse plugin.toml: {}", e),
            }
        })
    }

    /// Get the semver version.
    pub fn semver(&self) -> Option<Version> {
        Version::parse(&self.plugin.version).ok()
    }

    /// Verify the WASM file integrity.
    pub fn verify_checksum(&self, wasm_bytes: &[u8]) -> bool {
        match &self.plugin.checksum {
            Some(expected) => {
                let actual = blake3::hash(wasm_bytes).to_hex().to_string();
                actual == *expected
            }
            None => true, // No checksum = no verification
        }
    }
}
