use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
#[cfg(feature = "wasm")]
use wasmtime::*;

use crate::manifest::PluginManifest;
use claw_core::{Result, Tool, ToolCall, ToolResult};

/// A loaded plugin instance.
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub wasm_path: PathBuf,
    #[cfg(feature = "wasm")]
    module: Module,
}

/// The plugin host manages loading, sandboxing, and executing WASM plugins.
///
/// ## Plugin ABI
///
/// Plugins are WASM modules that export the following functions:
///
/// - `claw_malloc(size: u32) -> u32` — allocate `size` bytes in guest memory, return pointer
/// - `claw_invoke(ptr: u32, len: u32) -> u64` — invoke a tool call; input is JSON at `ptr`,
///   return value is `(result_ptr << 32) | result_len`
/// - `memory` — exported linear memory
///
/// ### Input JSON
/// ```json
/// { "tool": "tool_name", "arguments": { ... } }
/// ```
///
/// ### Output JSON
/// ```json
/// { "result": "text result", "data": { ... } }
/// ```
/// Or on error:
/// ```json
/// { "error": "error message" }
/// ```
pub struct PluginHost {
    #[cfg(feature = "wasm")]
    engine: Engine,
    plugins: HashMap<String, LoadedPlugin>,
    plugin_dir: PathBuf,
}

impl PluginHost {
    pub fn new(plugin_dir: &Path) -> Result<Self> {
        #[cfg(feature = "wasm")]
        {
            let mut config = Config::new();
            config.async_support(true);
            config.consume_fuel(true);

            let engine = Engine::new(&config).map_err(|e| claw_core::ClawError::Plugin {
                plugin: "host".into(),
                reason: format!("failed to create WASM engine: {}", e),
            })?;

            Ok(Self {
                engine,
                plugins: HashMap::new(),
                plugin_dir: plugin_dir.to_path_buf(),
            })
        }

        #[cfg(not(feature = "wasm"))]
        {
            Ok(Self {
                plugins: HashMap::new(),
                plugin_dir: plugin_dir.to_path_buf(),
            })
        }
    }

    /// Create an empty plugin host (for tests — no WASM engine needed).
    pub fn new_empty() -> Self {
        #[cfg(feature = "wasm")]
        {
            let mut config = Config::new();
            config.consume_fuel(true);
            let engine = Engine::new(&config).unwrap();
            Self {
                engine,
                plugins: HashMap::new(),
                plugin_dir: PathBuf::from("/tmp/claw-test-plugins"),
            }
        }

        #[cfg(not(feature = "wasm"))]
        {
            Self {
                plugins: HashMap::new(),
                plugin_dir: PathBuf::from("/tmp/claw-test-plugins"),
            }
        }
    }

    /// Scan the plugin directory and load all plugins.
    pub fn discover(&mut self) -> Result<Vec<String>> {
        let mut loaded = Vec::new();

        if !self.plugin_dir.exists() {
            info!(?self.plugin_dir, "plugin directory does not exist, skipping discovery");
            return Ok(loaded);
        }

        let entries =
            std::fs::read_dir(&self.plugin_dir).map_err(|e| claw_core::ClawError::Plugin {
                plugin: "host".into(),
                reason: format!("failed to read plugin dir: {}", e),
            })?;

        for entry in entries {
            let entry = entry.map_err(|e| claw_core::ClawError::Plugin {
                plugin: "host".into(),
                reason: e.to_string(),
            })?;

            let path = entry.path();
            if path.is_dir() {
                match self.load_from_dir(&path) {
                    Ok(name) => {
                        info!(plugin = %name, "loaded plugin");
                        loaded.push(name);
                    }
                    Err(e) => {
                        warn!(path = ?path, error = %e, "failed to load plugin");
                    }
                }
            }
        }

        Ok(loaded)
    }

    /// Load a plugin from a directory (must contain plugin.toml + *.wasm).
    fn load_from_dir(&mut self, dir: &Path) -> Result<String> {
        let manifest_path = dir.join("plugin.toml");
        let manifest_str =
            std::fs::read_to_string(&manifest_path).map_err(|e| claw_core::ClawError::Plugin {
                plugin: dir.display().to_string(),
                reason: format!("missing plugin.toml: {}", e),
            })?;

        let manifest = PluginManifest::from_toml(&manifest_str)?;
        let name = manifest.plugin.name.clone();

        // Find the .wasm file
        let wasm_path = dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            // Try any .wasm file in the directory
            let wasm_files: Vec<_> = std::fs::read_dir(dir)
                .map_err(|e| claw_core::ClawError::Plugin {
                    plugin: name.clone(),
                    reason: e.to_string(),
                })?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "wasm"))
                .collect();

            if wasm_files.is_empty() {
                return Err(claw_core::ClawError::Plugin {
                    plugin: name,
                    reason: "no .wasm file found".into(),
                });
            }
        }

        // Load and compile the module
        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| claw_core::ClawError::Plugin {
            plugin: name.clone(),
            reason: format!("failed to read wasm: {}", e),
        })?;

        // Verify checksum
        if !manifest.verify_checksum(&wasm_bytes) {
            return Err(claw_core::ClawError::Plugin {
                plugin: name,
                reason: "WASM checksum verification failed".into(),
            });
        }

        #[cfg(feature = "wasm")]
        let module =
            Module::new(&self.engine, &wasm_bytes).map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.clone(),
                reason: format!("failed to compile wasm: {}", e),
            })?;

        self.plugins.insert(
            name.clone(),
            LoadedPlugin {
                manifest,
                wasm_path,
                #[cfg(feature = "wasm")]
                module,
            },
        );

        Ok(name)
    }

    /// Get tools provided by all loaded plugins.
    pub fn tools(&self) -> Vec<Tool> {
        self.plugins
            .values()
            .flat_map(|plugin| {
                plugin.manifest.tools.iter().map(|t| Tool {
                    name: format!("{}.{}", plugin.manifest.plugin.name, t.name),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                    capabilities: vec![],
                    is_mutating: t.is_mutating,
                    risk_level: t.risk_level,
                    provider: Some(plugin.manifest.plugin.name.clone()),
                })
            })
            .collect()
    }

    /// Execute a tool call on a plugin.
    ///
    /// The plugin WASM module must export:
    /// - `memory` — linear memory
    /// - `claw_malloc(size: u32) -> u32` — allocate bytes
    /// - `claw_invoke(ptr: u32, len: u32) -> u64` — execute tool, return packed (ptr << 32 | len)
    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResult> {
        // Parse "plugin_name.tool_name" format
        let (plugin_name, tool_name) = call
            .tool_name
            .split_once('.')
            .ok_or_else(|| claw_core::ClawError::ToolNotFound(call.tool_name.clone()))?;

        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| claw_core::ClawError::Plugin {
                plugin: plugin_name.to_string(),
                reason: "plugin not loaded".into(),
            })?;

        // Verify the tool exists in the manifest
        if !plugin.manifest.tools.iter().any(|t| t.name == tool_name) {
            return Err(claw_core::ClawError::ToolNotFound(format!(
                "{}.{}",
                plugin_name, tool_name
            )));
        }

        debug!(
            plugin = plugin_name,
            tool = tool_name,
            "executing plugin tool"
        );

        #[cfg(feature = "wasm")]
        {
            self.execute_wasm(plugin, tool_name, &call.arguments, &call.id)
                .await
        }

        #[cfg(not(feature = "wasm"))]
        {
            let _ = (plugin, tool_name);
            Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "WASM support not enabled — rebuild with `--features wasm` to execute plugin '{}'",
                    plugin_name
                ),
                is_error: true,
                data: None,
            })
        }
    }

    /// Execute a tool via the WASM sandbox.
    #[cfg(feature = "wasm")]
    async fn execute_wasm(
        &self,
        plugin: &LoadedPlugin,
        tool_name: &str,
        arguments: &serde_json::Value,
        call_id: &str,
    ) -> Result<ToolResult> {
        let plugin_name = &plugin.manifest.plugin.name;

        // Serialize the invocation input
        let input = serde_json::json!({
            "tool": tool_name,
            "arguments": arguments,
        })
        .to_string();

        // Create a sandboxed store with fuel limits (prevents infinite loops)
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(10_000_000)
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("failed to set fuel: {}", e),
            })?;

        // Instantiate the module with an empty linker (sandboxed — no WASI imports)
        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate_async(&mut store, &plugin.module)
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("failed to instantiate WASM module: {}", e),
            })?;

        // Get required exports
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
            claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: "plugin does not export 'memory'".into(),
            }
        })?;

        let claw_malloc = instance
            .get_typed_func::<u32, u32>(&mut store, "claw_malloc")
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("missing export 'claw_malloc': {}", e),
            })?;

        let claw_invoke = instance
            .get_typed_func::<(u32, u32), u64>(&mut store, "claw_invoke")
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("missing export 'claw_invoke': {}", e),
            })?;

        // Allocate memory in guest for the input JSON
        let input_bytes = input.as_bytes();
        let input_ptr = claw_malloc
            .call_async(&mut store, input_bytes.len() as u32)
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("claw_malloc failed: {}", e),
            })?;

        // Write input into guest memory
        let mem_data = memory.data_mut(&mut store);
        let end = input_ptr as usize + input_bytes.len();
        if end > mem_data.len() {
            return Err(claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: "input exceeds guest memory bounds".into(),
            });
        }
        mem_data[input_ptr as usize..end].copy_from_slice(input_bytes);

        // Invoke the plugin
        let result_packed = claw_invoke
            .call_async(&mut store, (input_ptr, input_bytes.len() as u32))
            .await
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("claw_invoke failed: {}", e),
            })?;

        // Unpack result: high 32 bits = pointer, low 32 bits = length
        let result_ptr = (result_packed >> 32) as usize;
        let result_len = (result_packed & 0xFFFF_FFFF) as usize;

        // Read result from guest memory
        let mem_data = memory.data(&store);
        if result_ptr + result_len > mem_data.len() {
            return Err(claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: "result exceeds guest memory bounds".into(),
            });
        }

        let result_str = std::str::from_utf8(&mem_data[result_ptr..result_ptr + result_len])
            .map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("invalid UTF-8 in plugin result: {}", e),
            })?;

        // Parse the JSON result
        let result_json: serde_json::Value =
            serde_json::from_str(result_str).map_err(|e| claw_core::ClawError::Plugin {
                plugin: plugin_name.clone(),
                reason: format!("invalid JSON from plugin: {}", e),
            })?;

        let is_error = result_json.get("error").is_some();
        let content = if is_error {
            result_json["error"]
                .as_str()
                .unwrap_or("unknown plugin error")
                .to_string()
        } else {
            result_json
                .get("result")
                .map(|v| {
                    if v.is_string() {
                        v.as_str().unwrap().to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_else(|| result_str.to_string())
        };

        debug!(
            plugin = plugin_name.as_str(),
            tool = tool_name,
            is_error,
            "plugin invocation complete"
        );

        Ok(ToolResult {
            tool_call_id: call_id.to_string(),
            content,
            is_error,
            data: result_json.get("data").cloned(),
        })
    }

    /// List loaded plugins.
    pub fn loaded(&self) -> Vec<&PluginManifest> {
        self.plugins.values().map(|p| &p.manifest).collect()
    }

    /// Get a specific plugin's manifest by name.
    pub fn get_manifest(&self, name: &str) -> Option<&PluginManifest> {
        self.plugins.get(name).map(|p| &p.manifest)
    }

    /// Get the plugin directory path.
    pub fn plugin_dir(&self) -> &Path {
        &self.plugin_dir
    }

    /// Check if a specific plugin is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Unload a plugin.
    pub fn unload(&mut self, name: &str) -> bool {
        self.plugins.remove(name).is_some()
    }

    /// Uninstall a plugin — unload it and delete its directory.
    pub fn uninstall(&mut self, name: &str) -> Result<()> {
        self.plugins.remove(name);

        let plugin_path = self.plugin_dir.join(name);
        if plugin_path.exists() && plugin_path.is_dir() {
            std::fs::remove_dir_all(&plugin_path).map_err(|e| claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("failed to delete plugin directory: {}", e),
            })?;
            info!(plugin = name, path = ?plugin_path, "deleted plugin directory");
        } else {
            return Err(claw_core::ClawError::Plugin {
                plugin: name.to_string(),
                reason: format!("plugin directory not found: {}", plugin_path.display()),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_empty_creates_host() {
        let host = PluginHost::new_empty();
        assert!(host.loaded().is_empty());
        assert!(host.tools().is_empty());
    }

    #[test]
    fn discover_nonexistent_dir() {
        let mut host = PluginHost::new_empty();
        // Plugin dir doesn't exist — should return empty
        let loaded = host.discover().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn discover_real_dir_no_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = PluginHost::new(dir.path()).unwrap();
        let loaded = host.discover().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn tools_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        // Create a manifest
        let manifest = r#"
[plugin]
name = "test-plugin"
version = "1.0.0"
description = "A test plugin"

[[tools]]
name = "greet"
description = "Say hello"
risk_level = 1
is_mutating = false

[tools.parameters]
type = "object"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let parsed = PluginManifest::from_toml(manifest).unwrap();
        assert_eq!(parsed.plugin.name, "test-plugin");
        assert_eq!(parsed.tools.len(), 1);
        assert_eq!(parsed.tools[0].name, "greet");
    }

    #[test]
    fn is_loaded_and_unload() {
        let host = PluginHost::new_empty();
        assert!(!host.is_loaded("nonexistent"));
    }

    #[tokio::test]
    async fn execute_missing_plugin_errors() {
        let host = PluginHost::new_empty();
        let call = ToolCall {
            id: "call-1".into(),
            tool_name: "nonexistent.tool".into(),
            arguments: serde_json::json!({}),
        };
        let result = host.execute(&call).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_bad_format_errors() {
        let host = PluginHost::new_empty();
        let call = ToolCall {
            id: "call-1".into(),
            tool_name: "no_dot_in_name".into(),
            arguments: serde_json::json!({}),
        };
        let result = host.execute(&call).await;
        assert!(result.is_err());
    }

    #[test]
    fn get_manifest_returns_none_for_unknown() {
        let host = PluginHost::new_empty();
        assert!(host.get_manifest("unknown").is_none());
    }

    #[test]
    fn manifest_checksum_verification() {
        let manifest = PluginManifest::from_toml(
            r#"
[plugin]
name = "checksummed"
version = "1.0.0"
description = "test"
checksum = "0000000000000000000000000000000000000000000000000000000000000000"

[[tools]]
name = "t"
description = "t"
parameters = {}
"#,
        )
        .unwrap();

        // Wrong checksum should fail
        assert!(!manifest.verify_checksum(b"hello world"));
    }

    #[test]
    fn manifest_no_checksum_passes() {
        let manifest = PluginManifest::from_toml(
            r#"
[plugin]
name = "no-checksum"
version = "1.0.0"
description = "test"

[[tools]]
name = "t"
description = "t"
parameters = {}
"#,
        )
        .unwrap();

        // No checksum = always passes
        assert!(manifest.verify_checksum(b"anything"));
    }

    #[test]
    fn capabilities_parsing() {
        let manifest = PluginManifest::from_toml(
            r#"
[plugin]
name = "capable"
version = "1.0.0"
description = "test"

[capabilities]
network = ["https://api.example.com/*"]
filesystem = ["/tmp/safe/*"]
shell = true
host_functions = ["log"]

[[tools]]
name = "t"
description = "t"
parameters = {}
"#,
        )
        .unwrap();

        assert_eq!(
            manifest.capabilities.network,
            vec!["https://api.example.com/*"]
        );
        assert_eq!(manifest.capabilities.filesystem, vec!["/tmp/safe/*"]);
        assert!(manifest.capabilities.shell);
        assert_eq!(manifest.capabilities.host_functions, vec!["log"]);
    }

    #[test]
    fn uninstall_nonexistent_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = PluginHost::new(dir.path()).unwrap();
        assert!(host.uninstall("nonexistent").is_err());
    }
}
