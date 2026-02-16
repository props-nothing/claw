/// Check GitHub for a newer release.
/// Returns `Some((current, latest))` when an update is available.
pub async fn check_for_update() -> Option<(String, String)> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let client = reqwest::Client::builder()
        .user_agent("claw-update-check")
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let resp = client
        .get("https://api.github.com/repos/props-nothing/claw/releases/latest")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let tag = data["tag_name"].as_str()?;
    let latest = tag.strip_prefix('v').unwrap_or(tag).to_string();

    if version_newer(&latest, &current) {
        Some((current, latest))
    } else {
        None
    }
}

/// Simple semver comparison: is `a` newer than `b`?
fn version_newer(a: &str, b: &str) -> bool {
    let parse =
        |v: &str| -> Vec<u64> { v.split('.').filter_map(|s| s.parse::<u64>().ok()).collect() };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let xa = va.get(i).copied().unwrap_or(0);
        let xb = vb.get(i).copied().unwrap_or(0);
        if xa > xb {
            return true;
        }
        if xa < xb {
            return false;
        }
    }
    false
}

/// Perform the self-update: download the new binary and replace the current one.
pub async fn cmd_update(
    config: claw_config::ClawConfig,
    force: bool,
    no_restart: bool,
) -> claw_core::Result<()> {
    println!("🦞 Claw Update");
    println!("   Current version: v{}", env!("CARGO_PKG_VERSION"));
    println!();

    // 1. Check for update
    println!("   Checking for updates...");
    let (current, latest) = match check_for_update().await {
        Some(pair) => pair,
        None => {
            if force {
                println!("   No newer version found, but --force was specified.");
                let current = env!("CARGO_PKG_VERSION").to_string();
                (current.clone(), current)
            } else {
                println!("   ✅ Already up to date (v{}).", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
        }
    };

    if !force {
        println!("   🆕 New version available: v{current} → v{latest}");
    }

    // 2. Determine platform/target
    let target = detect_target()?;
    println!("   Target: {target}");

    // 3. Download new binary
    let filename = if cfg!(target_os = "windows") {
        format!("claw-v{latest}-{target}.exe")
    } else {
        format!("claw-v{latest}-{target}")
    };
    let url =
        format!("https://github.com/props-nothing/claw/releases/download/v{latest}/{filename}");

    println!("   Downloading {filename}...");

    let client = reqwest::Client::builder()
        .user_agent("claw-update")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| claw_core::ClawError::Agent(format!("HTTP client error: {e}")))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| claw_core::ClawError::Agent(format!("Failed to download update: {e}")))?;

    if !resp.status().is_success() {
        return Err(claw_core::ClawError::Agent(format!(
            "Download failed: HTTP {} — no prebuilt binary for {target}. \
             Try building from source: cargo install --git https://github.com/props-nothing/claw.git claw",
            resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| claw_core::ClawError::Agent(format!("Failed to read download: {e}")))?;

    println!("   Downloaded {} bytes", bytes.len());

    // 4. Find current binary location
    let current_exe = std::env::current_exe().map_err(|e| {
        claw_core::ClawError::Agent(format!("Cannot determine current binary path: {e}"))
    })?;

    // 5. Atomic replace: write to temp, rename over current
    let backup_path = current_exe.with_extension("old");
    let temp_path = current_exe.with_extension("new");

    std::fs::write(&temp_path, &bytes)
        .map_err(|e| claw_core::ClawError::Agent(format!("Failed to write new binary: {e}")))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| claw_core::ClawError::Agent(format!("Failed to set permissions: {e}")))?;
    }

    // Backup current binary
    if current_exe.exists() {
        let _ = std::fs::rename(&current_exe, &backup_path);
    }

    // Move new binary into place
    std::fs::rename(&temp_path, &current_exe).map_err(|e| {
        // Try to restore backup
        let _ = std::fs::rename(&backup_path, &current_exe);
        claw_core::ClawError::Agent(format!("Failed to replace binary (restored backup): {e}"))
    })?;

    // Clean up backup
    let _ = std::fs::remove_file(&backup_path);

    println!();
    println!("   ✅ Updated to v{latest}");

    // 6. Optionally restart the running agent
    if !no_restart {
        let listen = &config.server.listen;
        println!("   Requesting graceful restart...");
        match request_restart(listen).await {
            Ok(_) => println!("   ✅ Agent is restarting with the new version."),
            Err(e) => {
                println!("   ⚠️  Could not restart agent ({e}).");
                println!("      Restart manually: claw start");
            }
        }
    } else {
        println!("   Restart skipped (--no-restart). Run `claw start` to use the new version.");
    }

    Ok(())
}

/// Request a graceful restart via the API.
async fn request_restart(listen: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(format!("http://{listen}/api/v1/restart"))
        .send()
        .await
        .map_err(|e| format!("Agent not reachable at {listen}: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Restart request returned HTTP {}", resp.status()))
    }
}

/// Detect the current platform's target triple.
fn detect_target() -> claw_core::Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let target = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "arm") => "armv7-unknown-linux-gnueabihf",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => {
            return Err(claw_core::ClawError::Agent(format!(
                "No prebuilt binary for {os}/{arch}. \
                 Build from source: cargo install --git https://github.com/props-nothing/claw.git claw"
            )));
        }
    };

    Ok(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(version_newer("0.2.0", "0.1.0"));
        assert!(version_newer("1.0.0", "0.9.9"));
        assert!(version_newer("0.1.1", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.2.0"));
    }
}
