use std::path::Path;

use super::PluginAction;

pub(super) async fn cmd_plugin(
    config: claw_config::ClawConfig,
    action: PluginAction,
) -> claw_core::Result<()> {
    let host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
    let registry = claw_plugin::PluginRegistry::new(&config.plugins.registry_url);

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
            registry
                .install(&name, version.as_deref(), &config.plugins.plugin_dir)
                .await?;
            println!("✅ Installed {name}");
        }
        PluginAction::Uninstall { name } => {
            let mut host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
            host.uninstall(&name)?;
            println!("✅ Uninstalled {name}");
        }
        PluginAction::Search { query } => match registry.search(&query).await {
            Ok(results) => {
                for r in results {
                    println!(
                        "  {} v{} — {} ({} downloads)",
                        r.name, r.version, r.description, r.downloads
                    );
                }
            }
            Err(e) => {
                println!("Search failed: {e}");
            }
        },
        PluginAction::Info { name } => {
            let mut host = claw_plugin::PluginHost::new(&config.plugins.plugin_dir)?;
            let _ = host.discover();
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
                    // Capabilities
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
                    // Tools
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
            scaffold_plugin(&name, &config.plugins.plugin_dir)?;
        }
    }
    Ok(())
}

/// Scaffold a new plugin project with Cargo.toml, src/lib.rs, and plugin.toml.
fn scaffold_plugin(name: &str, plugin_dir: &Path) -> claw_core::Result<()> {
    let project_dir = plugin_dir.join(name);
    if project_dir.exists() {
        return Err(claw_core::ClawError::Plugin {
            plugin: name.to_string(),
            reason: format!("directory already exists: {}", project_dir.display()),
        });
    }

    std::fs::create_dir_all(project_dir.join("src"))?;

    // plugin.toml
    let manifest = format!(
        r#"[plugin]
name = "{name}"
version = "0.1.0"
description = "A Claw plugin"
authors = []

[capabilities]
# network = ["https://example.com/*"]
# filesystem = ["/tmp/{name}/*"]
# shell = false

[[tools]]
name = "hello"
description = "Say hello"
parameters = {{ "type": "object", "properties": {{ "name": {{ "type": "string" }} }} }}
risk_level = 0
is_mutating = false
"#
    );
    std::fs::write(project_dir.join("plugin.toml"), manifest)?;

    // Cargo.toml for the plugin crate
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
"#
    );
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

    // src/lib.rs — minimal plugin implementation
    let lib_rs = r##"//! Claw plugin — compiled to WebAssembly.
//!
//! Build with: cargo build --target wasm32-unknown-unknown --release

use std::alloc::{alloc, Layout};

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { alloc(layout) as u32 }
}

/// Main entry point — the host calls this with a JSON input.
/// Returns a packed u64: (result_ptr << 32) | result_len
#[unsafe(no_mangle)]
pub extern "C" fn claw_invoke(ptr: u32, len: u32) -> u64 {
    // Read the input JSON from host-provided memory
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let input: serde_json::Value = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(e) => return write_response(&format!(r#"{{"error":"bad input: {}"}}"#, e)),
    };

    let tool = input["tool"].as_str().unwrap_or("");
    let args = &input["arguments"];

    // Dispatch to tool implementations
    let result = match tool {
        "hello" => {
            let name = args["name"].as_str().unwrap_or("world");
            format!(r#"{{"result":"Hello, {}!"}}"#, name)
        }
        _ => format!(r#"{{"error":"unknown tool: {}"}}"#, tool),
    };

    write_response(&result)
}

fn write_response(json: &str) -> u64 {
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ((ptr as u64) << 32) | (bytes.len() as u64)
}
"##;
    std::fs::write(project_dir.join("src").join("lib.rs"), lib_rs)?;

    println!("✅ Created plugin scaffold at {}", project_dir.display());
    println!(
        "   Build: cd {} && cargo build --target wasm32-unknown-unknown --release",
        project_dir.display()
    );
    println!("   Then copy the .wasm file alongside plugin.toml");

    Ok(())
}
