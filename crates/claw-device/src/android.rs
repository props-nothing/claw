//! Android device control via ADB (Android Debug Bridge).
//!
//! Provides the agent with the ability to:
//! - List connected devices
//! - Install / launch apps
//! - Tap, swipe, type on screen
//! - Take screenshots
//! - Run shell commands on the device
//! - Record screen
//! - Push / pull files
//!
//! # Requirements
//!
//! ADB must be installed and on PATH. On macOS: `brew install android-platform-tools`.

use claw_core::ClawError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ─── Types ──────────────────────────────────────────────────────

/// Info about a connected Android device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub android_version: Option<String>,
}

/// Result of a screen capture.
#[derive(Debug, Clone)]
pub struct DeviceScreenshot {
    /// Base64-encoded PNG image data.
    pub data_base64: String,
}

/// Info about the current screen state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
    pub density: u32,
    pub current_activity: String,
    pub current_package: String,
}

/// Info about an installed app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub package: String,
    pub version: Option<String>,
}

// ─── ADB Bridge ──────────────────────────────────────────────────

/// Android Debug Bridge interface.
pub struct AndroidBridge {
    /// The serial of the active device (None = auto-select single device).
    active_device: Option<String>,
}

impl Default for AndroidBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidBridge {
    pub fn new() -> Self {
        Self {
            active_device: None,
        }
    }

    /// Run an ADB command and return stdout.
    async fn adb(&self, args: &[&str]) -> claw_core::Result<String> {
        let mut cmd = tokio::process::Command::new("adb");

        // Target specific device if set
        if let Some(ref serial) = self.active_device {
            cmd.arg("-s").arg(serial);
        }

        for arg in args {
            cmd.arg(arg);
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            cmd.output(),
        )
        .await
        .map_err(|_| ClawError::ToolExecution {
            tool: "android".into(),
            reason: "ADB command timed out".into(),
        })?
        .map_err(|e| ClawError::ToolExecution {
            tool: "android".into(),
            reason: format!("ADB not found or failed: {e}. Install with: brew install android-platform-tools"),
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ClawError::ToolExecution {
                tool: "android".into(),
                reason: format!("ADB error: {}", stderr.trim()),
            })
        }
    }

    /// Run a shell command on the Android device.
    async fn shell(&self, cmd: &str) -> claw_core::Result<String> {
        self.adb(&["shell", cmd]).await
    }

    // ── Public API ─────────────────────────────────────────────

    /// List connected Android devices.
    pub async fn list_devices(&self) -> claw_core::Result<Vec<AndroidDevice>> {
        let output = self.adb(&["devices", "-l"]).await?;
        let mut devices = Vec::new();

        for line in output.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let serial = parts[0].to_string();
                let state = parts[1].to_string();

                // Extract model from device info
                let model = parts
                    .iter()
                    .find(|p| p.starts_with("model:"))
                    .map(|p| p.strip_prefix("model:").unwrap_or("").to_string());

                devices.push(AndroidDevice {
                    serial,
                    state,
                    model,
                    android_version: None,
                });
            }
        }

        // Fetch Android version for connected devices
        for device in &mut devices {
            if device.state == "device"
                && let Ok(ver) = self.shell("getprop ro.build.version.release").await {
                    device.android_version = Some(ver.trim().to_string());
                }
        }

        Ok(devices)
    }

    /// Select a device by serial number.
    pub fn select_device(&mut self, serial: &str) {
        self.active_device = Some(serial.to_string());
    }

    /// Get info about the current screen.
    pub async fn screen_info(&self) -> claw_core::Result<ScreenInfo> {
        let size = self.shell("wm size").await.unwrap_or_default();
        let density = self.shell("wm density").await.unwrap_or_default();
        let activity = self
            .shell("dumpsys activity activities | grep mResumedActivity")
            .await
            .unwrap_or_default();

        // Parse "Physical size: 1080x2400"
        let (width, height) = size
            .trim()
            .split_once(": ")
            .and_then(|(_, s)| s.split_once('x'))
            .map(|(w, h)| {
                (
                    w.trim().parse::<u32>().unwrap_or(1080),
                    h.trim().parse::<u32>().unwrap_or(1920),
                )
            })
            .unwrap_or((1080, 1920));

        // Parse "Physical density: 420"
        let density_val = density
            .trim()
            .split_once(": ")
            .map(|(_, d)| d.trim().parse::<u32>().unwrap_or(420))
            .unwrap_or(420);

        // Parse current activity
        let (package, act) = activity
            .trim()
            .rsplit_once(' ')
            .and_then(|(_, comp)| comp.split_once('/'))
            .map(|(p, a)| (p.to_string(), format!("{}/{}", p, a.trim_end_matches('}'))))
            .unwrap_or(("unknown".into(), "unknown".into()));

        Ok(ScreenInfo {
            width,
            height,
            density: density_val,
            current_activity: act,
            current_package: package,
        })
    }

    /// Take a screenshot and return it as base64 PNG.
    pub async fn screenshot(&self) -> claw_core::Result<DeviceScreenshot> {
        // Capture to device, pull to stdout as base64
        let output = self.adb(&["exec-out", "screencap", "-p"]).await;

        match output {
            Ok(raw) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
                Ok(DeviceScreenshot { data_base64: b64 })
            }
            Err(_) => {
                // Fallback: capture to file, pull, read, delete
                self.shell("screencap -p /sdcard/claw_screenshot.png")
                    .await?;
                let _pull = self
                    .adb(&[
                        "pull",
                        "/sdcard/claw_screenshot.png",
                        "/tmp/claw_android_screenshot.png",
                    ])
                    .await?;
                self.shell("rm /sdcard/claw_screenshot.png").await?;

                let bytes = tokio::fs::read("/tmp/claw_android_screenshot.png")
                    .await
                    .map_err(|e| ClawError::ToolExecution {
                        tool: "android".into(),
                        reason: format!("failed to read screenshot: {e}"),
                    })?;
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let _ = tokio::fs::remove_file("/tmp/claw_android_screenshot.png").await;
                Ok(DeviceScreenshot { data_base64: b64 })
            }
        }
    }

    /// Tap at screen coordinates.
    pub async fn tap(&self, x: u32, y: u32) -> claw_core::Result<String> {
        self.shell(&format!("input tap {x} {y}")).await?;
        Ok(format!("tapped ({x}, {y})"))
    }

    /// Swipe from one point to another.
    pub async fn swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u32,
    ) -> claw_core::Result<String> {
        self.shell(&format!("input swipe {x1} {y1} {x2} {y2} {duration_ms}"))
            .await?;
        Ok(format!(
            "swiped ({x1},{y1}) → ({x2},{y2}) over {duration_ms}ms"
        ))
    }

    /// Type text on the device.
    pub async fn type_text(&self, text: &str) -> claw_core::Result<String> {
        // ADB input text doesn't handle spaces well, use key events for spaces
        let escaped = text
            .replace(' ', "%s")
            .replace('&', "\\&")
            .replace('\'', "\\'");
        self.shell(&format!("input text '{escaped}'")).await?;
        Ok(format!("typed {} chars", text.len()))
    }

    /// Press a key (home, back, enter, etc.).
    pub async fn press_key(&self, key: &str) -> claw_core::Result<String> {
        let lower = key.to_lowercase();
        let keycode = match lower.as_str() {
            "home" => "KEYCODE_HOME",
            "back" => "KEYCODE_BACK",
            "enter" | "return" => "KEYCODE_ENTER",
            "tab" => "KEYCODE_TAB",
            "menu" | "recent" | "recents" => "KEYCODE_APP_SWITCH",
            "power" => "KEYCODE_POWER",
            "volume_up" => "KEYCODE_VOLUME_UP",
            "volume_down" => "KEYCODE_VOLUME_DOWN",
            "delete" | "backspace" => "KEYCODE_DEL",
            "escape" | "esc" => "KEYCODE_ESCAPE",
            other => other, // Pass through raw keycode
        };
        self.shell(&format!("input keyevent {keycode}")).await?;
        Ok(format!("pressed {key}"))
    }

    /// Install an APK.
    pub async fn install(&self, apk_path: &str) -> claw_core::Result<String> {
        let output = self.adb(&["install", "-r", apk_path]).await?;
        Ok(output.trim().to_string())
    }

    /// Launch an app by package name.
    pub async fn launch_app(&self, package: &str) -> claw_core::Result<String> {
        let _output = self
            .shell(&format!(
                "monkey -p {package} -c android.intent.category.LAUNCHER 1"
            ))
            .await?;
        Ok(format!("launched {package}"))
    }

    /// Force stop an app.
    pub async fn stop_app(&self, package: &str) -> claw_core::Result<String> {
        self.shell(&format!("am force-stop {package}")).await?;
        Ok(format!("stopped {package}"))
    }

    /// List installed packages.
    pub async fn list_apps(&self) -> claw_core::Result<Vec<AppInfo>> {
        let output = self.shell("pm list packages -3").await?;
        let apps: Vec<AppInfo> = output
            .lines()
            .filter_map(|line| {
                line.strip_prefix("package:").map(|pkg| AppInfo {
                    package: pkg.trim().to_string(),
                    version: None,
                })
            })
            .collect();
        Ok(apps)
    }

    /// Run a shell command on the device.
    pub async fn run_shell(&self, command: &str) -> claw_core::Result<String> {
        self.shell(command).await
    }

    /// Push a file to the device.
    pub async fn push_file(&self, local: &str, remote: &str) -> claw_core::Result<String> {
        self.adb(&["push", local, remote]).await
    }

    /// Pull a file from the device.
    pub async fn pull_file(&self, remote: &str, local: &str) -> claw_core::Result<String> {
        self.adb(&["pull", remote, local]).await
    }

    /// Get the UI hierarchy as XML (for accessibility-based automation).
    pub async fn dump_ui(&self) -> claw_core::Result<String> {
        self.shell("uiautomator dump /sdcard/claw_ui.xml && cat /sdcard/claw_ui.xml && rm /sdcard/claw_ui.xml").await
    }

    /// Get device status.
    pub async fn status(&self) -> claw_core::Result<Value> {
        let devices = self.list_devices().await.unwrap_or_default();
        let has_device = devices.iter().any(|d| d.state == "device");

        Ok(json!({
            "adb_available": check_adb_available().await,
            "devices": devices,
            "active_device": self.active_device,
            "connected": has_device,
        }))
    }
}

/// Check if ADB is available on the system.
async fn check_adb_available() -> bool {
    tokio::process::Command::new("adb")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
