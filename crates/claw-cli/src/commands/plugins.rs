use std::path::{Path, PathBuf};

use super::PluginAction;

pub(super) async fn cmd_plugin(
    config: claw_config::ClawConfig,
    action: PluginAction,
) -> claw_core::Result<()> {
    let mut host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
    let _ = host.discover();
    let registry_url = config
        .plugins
        .effective_registry_url(config.services.hub_url.as_deref())
        .unwrap_or_default();
    let registry = claw_plugin::PluginRegistry::new(&registry_url);

    match action {
        PluginAction::List => {
            let plugins = host.loaded();
            if plugins.is_empty() {
                println!("No plugins installed.");
            } else {
                for p in plugins {
                    println!(
                        "  {} v{} — {}",
                        p.plugin.name, p.plugin.version, p.plugin.description
                    );
                }
            }
        }
        PluginAction::Install { name, version } => {
            if registry_url.is_empty() {
                println!("No hub configured. Set plugins.registry_url or services.hub_url in claw.toml,");
                println!("or run 'claw hub serve' to host your own hub.");
                return Ok(());
            }
            registry
                .install(&name, version.as_deref(), &config.plugins.plugin_dir)
                .await?;
            println!("✅ Installed {name}");
        }
        PluginAction::Uninstall { name } => {
            host.uninstall(&name)?;
            println!("✅ Uninstalled {name}");
        }
        PluginAction::Search { query } => {
            if registry_url.is_empty() {
                println!("No hub configured. Set plugins.registry_url or services.hub_url in claw.toml,");
                println!("or run 'claw hub serve' to host your own hub.");
                return Ok(());
            }
            match registry.search(&query).await {
                Ok(results) => {
                    if results.is_empty() {
                        println!("No plugins found matching '{query}'.");
                    } else {
                        for r in results {
                            println!(
                                "  {} v{} — {} ({} downloads)",
                                r.name, r.version, r.description, r.downloads
                            );
                        }
                    }
                }
                Err(e) => {
                    println!("Search failed: {e}");
                }
            }
        }
        PluginAction::Info { name } => {
            match host.get_manifest(&name) {
                Some(manifest) => {
                    println!(
                        "\x1b[1m{}\x1b[0m v{}",
                        manifest.plugin.name, manifest.plugin.version
                    );
                    println!("  {}", manifest.plugin.description);
                    if !manifest.plugin.authors.is_empty() {
                        println!("  Authors: {}", manifest.plugin.authors.join(", "));
                    }
                    if let Some(ref license) = manifest.plugin.license {
                        println!("  License: {license}");
                    }
                    if let Some(ref homepage) = manifest.plugin.homepage {
                        println!("  Homepage: {homepage}");
                    }
                    if let Some(ref checksum) = manifest.plugin.checksum {
                        println!("  Checksum: {}", &checksum[..checksum.len().min(16)]);
                    }
                    let caps = &manifest.capabilities;
                    if !caps.network.is_empty() || !caps.filesystem.is_empty() || caps.shell {
                        println!("\n  \x1b[1mCapabilities:\x1b[0m");
                        if !caps.network.is_empty() {
                            println!("    Network: {}", caps.network.join(", "));
                        }
                        if !caps.filesystem.is_empty() {
                            println!("    Filesystem: {}", caps.filesystem.join(", "));
                        }
                        if caps.shell {
                            println!("    Shell: yes");
                        }
                    }
                    if !manifest.tools.is_empty() {
                        println!("\n  \x1b[1mTools ({}):\x1b[0m", manifest.tools.len());
                        for tool in &manifest.tools {
                            let risk = if tool.risk_level > 0 {
                                format!(" [risk={}]", tool.risk_level)
                            } else {
                                String::new()
                            };
                            let mutating = if tool.is_mutating { " ✏️" } else { "" };
                            println!(
                                "    {}{}{} — {}",
                                tool.name, mutating, risk, tool.description
                            );
                        }
                    }
                }
                None => {
                    println!("Plugin '{name}' not found. Is it installed?");
                }
            }
        }
        PluginAction::Create { name } => {
            scaffold_plugin(&name)?;
        }
        PluginAction::Build { path } => {
            build_plugin(&path, &config.plugins.plugin_dir)?;
        }
    }
    Ok(())
}

// ── Scaffold ──────────────────────────────────────────────────

/// Scaffold a new plugin project in the current directory.
/// Creates `<name>/` with Cargo.toml, plugin.toml, and src/lib.rs.
fn scaffold_plugin(name: &str) -> claw_core::Result<()> {
    let project_dir = PathBuf::from(name);
    if project_dir.exists() {
        return Err(claw_core::ClawError::Plugin {
            plugin: name.to_string(),
            reason: format!("directory already exists: {}", project_dir.display()),
        });
    }

    std::fs::create_dir_all(project_dir.join("src"))?;

    // plugin.toml — valid TOML with proper table syntax for parameters
    let manifest = format!(
        r#"[plugin]
name = "{name}"
version = "0.1.0"
description = "A Claw plugin"
authors = []
license = "MIT"

[capabilities]
# Uncomment to grant permissions:
# network = ["https://example.com/*"]
# filesystem = ["/tmp/{name}/*"]
# shell = false

[[tools]]
name = "hello"
description = "Say hello to someone"
risk_level = 0
is_mutating = false

[tools.parameters]
type = "object"

[tools.parameters.properties.name]
type = "string"
description = "Name to greet"
"#
    );
    std::fs::write(project_dir.join("plugin.toml"), manifest)?;

    // Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[profile.release]
opt-level = "z"
lto = true
strip = true
"#
    );
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

    // src/lib.rs — clean working template using serde_json::json!
    let lib_rs = format!(
        r#"//! {name} — Claw WASM plugin.
//!
//! Build & install:  claw plugin build
//! Or manually:      cargo build --target wasm32-unknown-unknown --release

use std::alloc::{{alloc, Layout}};

// ── WASM ABI exports ──────────────────────────────────────────

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {{
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe {{ alloc(layout) as u32 }}
}}

/// Main entry point — called by the host with JSON input.
///
/// Input:  {{"tool": "name", "arguments": {{...}}}}
/// Output: {{"result": "text"}} or {{"error": "msg"}}
///
/// Returns packed u64: (result_ptr << 32) | result_len
#[unsafe(no_mangle)]
pub extern "C" fn claw_invoke(ptr: u32, len: u32) -> u64 {{
    let input_bytes = unsafe {{ std::slice::from_raw_parts(ptr as *const u8, len as usize) }};
    let input: serde_json::Value = match serde_json::from_slice(input_bytes) {{
        Ok(v) => v,
        Err(e) => {{
            return write_json(&serde_json::json!({{"error": format!("bad input: {{e}}")}}));
        }}
    }};

    let tool = input["tool"].as_str().unwrap_or("");
    let args = &input["arguments"];

    let result = match tool {{
        "hello" => {{
            let name = args["name"].as_str().unwrap_or("world");
            serde_json::json!({{"result": format!("Hello, {{name}}! 🦞")}})
        }}
        // Add more tools here:
        // "my_tool" => {{
        //     serde_json::json!({{"result": "done"}})
        // }}
        _ => serde_json::json!({{"error": format!("unknown tool: {{tool}}")}}),
    }};

    write_json(&result)
}}

// ── Helpers ───────────────────────────────────────────────────

fn write_json(value: &serde_json::Value) -> u64 {{
    let json = serde_json::to_string(value).unwrap();
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe {{ alloc(layout) }};
    unsafe {{ std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) }};
    ((ptr as u64) << 32) | (bytes.len() as u64)
}}
"#
    );
    std::fs::write(project_dir.join("src").join("lib.rs"), lib_rs)?;

    // .gitignore
    std::fs::write(project_dir.join(".gitignore"), "/target\n")?;

    let abs_path = std::env::current_dir()
        .map(|d| d.join(name))
        .unwrap_or(project_dir);

    println!("✅ Created plugin project: {}", abs_path.display());
    println!();
    println!("  Next steps:");
    println!("    cd {name}");
    println!("    # edit src/lib.rs and plugin.toml");
    println!("    claw plugin build          # builds WASM and installs the plugin");
    println!("    claw plugin list            # verify it's loaded");
    println!();

    Ok(())
}

// ── Build & install ───────────────────────────────────────────

/// Build a plugin from source and install it into the plugin directory.
/// Runs `cargo build --target wasm32-unknown-unknown --release`, then
/// copies the .wasm and plugin.toml to ~/.claw/plugins/<name>/.
fn build_plugin(path: &str, plugin_dir: &Path) -> claw_core::Result<()> {
    let project_dir = PathBuf::from(path).canonicalize().map_err(|e| {
        claw_core::ClawError::Plugin {
            plugin: path.to_string(),
            reason: format!("invalid path: {e}"),
        }
    })?;

    // Validate: must have plugin.toml
    let manifest_path = project_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(claw_core::ClawError::Plugin {
            plugin: path.to_string(),
            reason: format!(
                "not a plugin project: no plugin.toml found in {}",
                project_dir.display()
            ),
        });
    }

    // Parse manifest to get the plugin name
    let manifest_str = std::fs::read_to_string(&manifest_path)?;
    let manifest = claw_plugin::PluginManifest::from_toml(&manifest_str)?;
    let name = &manifest.plugin.name;
    let crate_name = name.replace('-', "_");

    println!("🔨 Building {name} v{} …", manifest.plugin.version);

    // Run cargo build
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ])
        .current_dir(&project_dir)
        .status()
        .map_err(|e| claw_core::ClawError::Plugin {
            plugin: name.clone(),
            reason: format!("failed to run cargo: {e}"),
        })?;

    if !status.success() {
        return Err(claw_core::ClawError::Plugin {
            plugin: name.clone(),
            reason: "cargo build failed".into(),
        });
    }

    // Find the built .wasm file
    let wasm_src = project_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{crate_name}.wasm"));

    if !wasm_src.exists() {
        return Err(claw_core::ClawError::Plugin {
            plugin: name.clone(),
            reason: format!("built .wasm not found at {}", wasm_src.display()),
        });
    }

    let wasm_size = std::fs::metadata(&wasm_src)
        .map(|m| m.len())
        .unwrap_or(0);

    // Install: copy plugin.toml + .wasm to the plugins directory
    let dest_dir = plugin_dir.join(name);
    std::fs::create_dir_all(&dest_dir)?;
    std::fs::copy(&manifest_path, dest_dir.join("plugin.toml"))?;
    std::fs::copy(&wasm_src, dest_dir.join(format!("{crate_name}.wasm")))?;

    println!("✅ Installed {name} → {}", dest_dir.display());
    println!(
        "   WASM size: {}",
        format_bytes(wasm_size)
    );
    println!(
        "   Tools: {}",
        manifest
            .tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
