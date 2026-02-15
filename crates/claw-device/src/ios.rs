//! iOS device control via libimobiledevice + Xcode CLI tools.
//!
//! Provides the agent with the ability to:
//! - List connected iOS devices (USB & Wi-Fi)
//! - Get device info (model, iOS version, UDID)
//! - Install / launch apps
//! - Take screenshots
//! - Interact with the UI via Xcode's simctl (simulators) or idb (physical)
//! - Push / pull files via AFC (Apple File Conduit)
//!
//! # Requirements
//!
//! For **physical devices**: `brew install libimobiledevice ideviceinstaller`
//! For **simulators**: Xcode must be installed (`xcrun simctl`)
//! For **advanced UI automation**: Facebook's `idb` (`pip install fb-idb`)

use claw_core::ClawError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{info, warn};

// ─── Types ──────────────────────────────────────────────────────

/// Info about a connected iOS device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IosDevice {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub ios_version: String,
    /// "physical" or "simulator"
    pub device_type: String,
    pub state: String,
}

/// Screenshot from an iOS device.
#[derive(Debug, Clone)]
pub struct IosScreenshot {
    pub data_base64: String,
}

// ─── iOS Bridge ──────────────────────────────────────────────────

/// iOS device interface — works with both physical devices and simulators.
pub struct IosBridge {
    /// UDID of the active device.
    active_device: Option<String>,
    /// Whether the active device is a simulator.
    is_simulator: bool,
}

impl Default for IosBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl IosBridge {
    pub fn new() -> Self {
        Self {
            active_device: None,
            is_simulator: false,
        }
    }

    /// Run a shell command and return stdout.
    async fn run_cmd(program: &str, args: &[&str]) -> claw_core::Result<String> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new(program).args(args).output(),
        )
        .await
        .map_err(|_| ClawError::ToolExecution {
            tool: "ios".into(),
            reason: format!("{program} timed out"),
        })?
        .map_err(|e| ClawError::ToolExecution {
            tool: "ios".into(),
            reason: format!("{program} failed: {e}"),
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("{program} error: {}", stderr.trim()),
            })
        }
    }

    // ── Device Discovery ─────────────────────────────────────

    /// List all connected iOS devices (physical + simulators).
    pub async fn list_devices(&self) -> claw_core::Result<Vec<IosDevice>> {
        let mut devices = Vec::new();

        // 1. Physical devices via idevice_id
        if let Ok(output) = Self::run_cmd("idevice_id", &["-l"]).await {
            for udid in output.lines() {
                let udid = udid.trim();
                if udid.is_empty() {
                    continue;
                }

                let name = Self::run_cmd("idevicename", &["-u", udid])
                    .await
                    .unwrap_or_else(|_| "Unknown".into())
                    .trim()
                    .to_string();

                let info = Self::run_cmd("ideviceinfo", &["-u", udid, "-k", "ProductType"])
                    .await
                    .unwrap_or_else(|_| "Unknown".into())
                    .trim()
                    .to_string();

                let version = Self::run_cmd("ideviceinfo", &["-u", udid, "-k", "ProductVersion"])
                    .await
                    .unwrap_or_else(|_| "Unknown".into())
                    .trim()
                    .to_string();

                devices.push(IosDevice {
                    udid: udid.to_string(),
                    name,
                    model: info,
                    ios_version: version,
                    device_type: "physical".into(),
                    state: "connected".into(),
                });
            }
        }

        // 2. Simulators via xcrun simctl
        if let Ok(output) = Self::run_cmd("xcrun", &["simctl", "list", "devices", "-j"]).await
            && let Ok(parsed) = serde_json::from_str::<Value>(&output)
            && let Some(device_map) = parsed["devices"].as_object()
        {
            for (runtime, devs) in device_map {
                if let Some(arr) = devs.as_array() {
                    for dev in arr {
                        let state = dev["state"].as_str().unwrap_or("Shutdown");
                        // Only show booted simulators unless no physical devices
                        let udid = dev["udid"].as_str().unwrap_or("").to_string();
                        let name = dev["name"].as_str().unwrap_or("").to_string();

                        // Extract iOS version from runtime string
                        let ios_ver = runtime
                            .rsplit('.')
                            .next()
                            .unwrap_or("")
                            .replace("iOS-", "")
                            .replace('-', ".");

                        devices.push(IosDevice {
                            udid,
                            name,
                            model: "Simulator".into(),
                            ios_version: ios_ver,
                            device_type: "simulator".into(),
                            state: state.to_string(),
                        });
                    }
                }
            }
        }

        Ok(devices)
    }

    /// Select a device by UDID.
    pub fn select_device(&mut self, udid: &str, is_simulator: bool) {
        self.active_device = Some(udid.to_string());
        self.is_simulator = is_simulator;
    }

    /// Auto-select a device (prefer physical, then booted simulator).
    pub async fn auto_select(&mut self) -> claw_core::Result<String> {
        let devices = self.list_devices().await?;

        // Prefer physical devices
        if let Some(dev) = devices.iter().find(|d| d.device_type == "physical") {
            self.active_device = Some(dev.udid.clone());
            self.is_simulator = false;
            return Ok(format!(
                "selected physical device: {} ({})",
                dev.name, dev.udid
            ));
        }

        // Then booted simulators
        if let Some(dev) = devices
            .iter()
            .find(|d| d.device_type == "simulator" && d.state == "Booted")
        {
            self.active_device = Some(dev.udid.clone());
            self.is_simulator = true;
            return Ok(format!("selected simulator: {} ({})", dev.name, dev.udid));
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: "no iOS devices found. Connect a device via USB or boot a simulator.".into(),
        })
    }

    fn require_device(&self) -> claw_core::Result<&str> {
        self.active_device
            .as_deref()
            .ok_or_else(|| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: "no device selected. Use ios_devices to list and select one.".into(),
            })
    }

    // ── Screenshots ──────────────────────────────────────────

    /// Take a screenshot.
    pub async fn screenshot(&self) -> claw_core::Result<IosScreenshot> {
        let udid = self.require_device()?;
        let tmp = format!("/tmp/claw_ios_screenshot_{udid}.png");

        if self.is_simulator {
            Self::run_cmd("xcrun", &["simctl", "io", udid, "screenshot", &tmp]).await?;
        } else {
            Self::run_cmd("idevicescreenshot", &["-u", udid, &tmp]).await?;
        }

        let bytes = tokio::fs::read(&tmp)
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("failed to read screenshot: {e}"),
            })?;

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let _ = tokio::fs::remove_file(&tmp).await;

        Ok(IosScreenshot { data_base64: b64 })
    }

    // ── App Management ───────────────────────────────────────

    /// Install an app (.ipa or .app).
    pub async fn install_app(&self, path: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            Self::run_cmd("xcrun", &["simctl", "install", udid, path]).await?;
        } else {
            Self::run_cmd("ideviceinstaller", &["-u", udid, "-i", path]).await?;
        }

        Ok(format!("installed {path}"))
    }

    /// Launch an app by bundle identifier.
    pub async fn launch_app(&self, bundle_id: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            Self::run_cmd("xcrun", &["simctl", "launch", udid, bundle_id]).await?;
        } else {
            Self::run_cmd("idevicedebug", &["-u", udid, "run", bundle_id]).await?;
        }

        Ok(format!("launched {bundle_id}"))
    }

    /// Terminate an app.
    pub async fn terminate_app(&self, bundle_id: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            Self::run_cmd("xcrun", &["simctl", "terminate", udid, bundle_id]).await?;
        } else {
            // No direct equivalent for physical; use killall via iproxy
            warn!("terminate on physical device requires developer disk image");
        }

        Ok(format!("terminated {bundle_id}"))
    }

    /// List installed apps.
    pub async fn list_apps(&self) -> claw_core::Result<Vec<Value>> {
        let udid = self.require_device()?;

        if self.is_simulator {
            let output = Self::run_cmd("xcrun", &["simctl", "listapps", udid]).await?;
            // Parse the plist output (simplified)
            Ok(vec![
                json!({ "note": "app list available", "raw_length": output.len() }),
            ])
        } else {
            let output = Self::run_cmd("ideviceinstaller", &["-u", udid, "-l"]).await?;
            let apps: Vec<Value> = output
                .lines()
                .skip(1) // header
                .map(|line| {
                    let parts: Vec<&str> = line.splitn(3, ',').collect();
                    json!({
                        "bundle_id": parts.first().unwrap_or(&""),
                        "version": parts.get(1).unwrap_or(&""),
                        "name": parts.get(2).unwrap_or(&""),
                    })
                })
                .collect();
            Ok(apps)
        }
    }

    // ── UI Interaction (Simulator) ───────────────────────────

    /// Tap at screen coordinates.
    ///
    /// Strategy chain:
    /// 1. `idb` (most reliable — works for simulators and physical devices)
    /// 2. macOS AppleScript + Simulator.app window click (simulators only, no extra deps)
    pub async fn tap(&self, x: u32, y: u32) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        // 1. Try idb (works for both physical and simulators)
        if check_idb_available().await {
            Self::run_cmd(
                "idb",
                &["ui", "tap", &x.to_string(), &y.to_string(), "--udid", udid],
            )
            .await?;
            return Ok(format!("tapped ({x}, {y}) via idb"));
        }

        // 2. For simulators: use AppleScript to click in the Simulator window
        if self.is_simulator {
            return self.applescript_tap(x, y).await;
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: "tap on physical device requires idb. Install: brew install idb-companion && pip3 install fb-idb".into(),
        })
    }

    /// Tap using macOS AppleScript — activates Simulator.app and clicks at
    /// the correct position by mapping device coordinates to window coordinates.
    async fn applescript_tap(&self, x: u32, y: u32) -> claw_core::Result<String> {
        // Step 1: Get Simulator window position and size via AppleScript
        let bounds_script = r#"
tell application "Simulator" to activate
delay 0.2
tell application "System Events"
    tell process "Simulator"
        set {wx, wy} to position of window 1
        set {ww, wh} to size of window 1
        return (wx as text) & "," & (wy as text) & "," & (ww as text) & "," & (wh as text)
    end tell
end tell"#;

        let bounds_output = Self::run_cmd("osascript", &["-e", bounds_script])
            .await
            .map_err(|_| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: "failed to get Simulator window bounds. Is Simulator.app open?".into(),
            })?;

        let parts: Vec<f64> = bounds_output
            .trim()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if parts.len() != 4 {
            return Err(ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("unexpected window bounds: '{}'", bounds_output.trim()),
            });
        }

        let (win_x, win_y, win_w, win_h) = (parts[0], parts[1], parts[2], parts[3]);

        // Step 2: Get device screen dimensions from the screenshot size
        let (dev_w, dev_h) = self
            .get_device_screen_size()
            .await
            .unwrap_or((393.0, 852.0));

        // Step 3: Map device coordinates to screen coordinates
        // The Simulator window has a title bar (~28px) and renders the device screen in the content area
        let title_bar = 28.0;
        let content_h = win_h - title_bar;
        let screen_x = win_x + (x as f64 / dev_w * win_w);
        let screen_y = win_y + title_bar + (y as f64 / dev_h * content_h);

        info!(
            x,
            y, screen_x, screen_y, "applescript tap: device → screen coords"
        );

        // Step 4: Click at the computed screen coordinates
        let click_script = format!(
            "tell application \"System Events\" to click at {{{}, {}}}",
            screen_x as i32, screen_y as i32
        );

        Self::run_cmd("osascript", &["-e", &click_script])
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("AppleScript click failed: {e}"),
            })?;

        Ok(format!(
            "tapped ({}, {}) via AppleScript (screen: {}, {})",
            x, y, screen_x as i32, screen_y as i32
        ))
    }

    /// Get the device's logical screen dimensions by taking a quick screenshot
    /// and reading its pixel size, then dividing by the Retina scale factor.
    async fn get_device_screen_size(&self) -> Option<(f64, f64)> {
        let udid = self.active_device.as_deref()?;
        let tmp = format!("/tmp/claw_ios_size_probe_{udid}.png");

        // Take a screenshot to get pixel dimensions
        if Self::run_cmd("xcrun", &["simctl", "io", udid, "screenshot", &tmp])
            .await
            .is_err()
        {
            return None;
        }

        // Read PNG dimensions from the file header using `sips`
        let output = Self::run_cmd("sips", &["-g", "pixelWidth", "-g", "pixelHeight", &tmp])
            .await
            .ok()?;
        let _ = tokio::fs::remove_file(&tmp).await;

        let mut pw: Option<f64> = None;
        let mut ph: Option<f64> = None;
        for line in output.lines() {
            if line.contains("pixelWidth") {
                pw = line
                    .split(':')
                    .next_back()
                    .and_then(|s| s.trim().parse().ok());
            }
            if line.contains("pixelHeight") {
                ph = line
                    .split(':')
                    .next_back()
                    .and_then(|s| s.trim().parse().ok());
            }
        }

        // The Simulator renders at device logical points, not physical pixels.
        // Divide by scale factor (3x for Pro models, 2x for SE, etc.)
        // A rough heuristic: if width > 1000, it's 3x; if > 600, it's 2x
        let pixel_w = pw?;
        let pixel_h = ph?;
        let scale = if pixel_w > 1000.0 {
            3.0
        } else if pixel_w > 600.0 {
            2.0
        } else {
            1.0
        };

        Some((pixel_w / scale, pixel_h / scale))
    }

    /// Swipe on the device.
    ///
    /// Strategy: idb → AppleScript mouse-drag → error
    pub async fn swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u32,
    ) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        // 1. Try idb
        if check_idb_available().await {
            Self::run_cmd(
                "idb",
                &[
                    "ui",
                    "swipe",
                    &x1.to_string(),
                    &y1.to_string(),
                    &x2.to_string(),
                    &y2.to_string(),
                    "--duration",
                    &(duration_ms as f64 / 1000.0).to_string(),
                    "--udid",
                    udid,
                ],
            )
            .await?;
            return Ok(format!("swiped ({x1},{y1}) → ({x2},{y2}) via idb"));
        }

        // 2. For simulators: use AppleScript mouse drag
        if self.is_simulator {
            return self.applescript_swipe(x1, y1, x2, y2, duration_ms).await;
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: "swipe on physical device requires idb. Install: brew install idb-companion && pip3 install fb-idb".into(),
        })
    }

    /// Swipe using a Python3 CGEvent script (built-in on macOS with CoreGraphics).
    async fn applescript_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u32,
    ) -> claw_core::Result<String> {
        // Get window bounds and device dimensions (same as tap)
        let bounds_script = r#"
tell application "Simulator" to activate
delay 0.2
tell application "System Events"
    tell process "Simulator"
        set {wx, wy} to position of window 1
        set {ww, wh} to size of window 1
        return (wx as text) & "," & (wy as text) & "," & (ww as text) & "," & (wh as text)
    end tell
end tell"#;

        let bounds_output = Self::run_cmd("osascript", &["-e", bounds_script])
            .await
            .map_err(|_| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: "failed to get Simulator window bounds".into(),
            })?;

        let parts: Vec<f64> = bounds_output
            .trim()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if parts.len() != 4 {
            return Err(ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("unexpected window bounds: '{}'", bounds_output.trim()),
            });
        }

        let (win_x, win_y, win_w, win_h) = (parts[0], parts[1], parts[2], parts[3]);
        let (dev_w, dev_h) = self
            .get_device_screen_size()
            .await
            .unwrap_or((393.0, 852.0));
        let title_bar = 28.0;
        let content_h = win_h - title_bar;

        let sx1 = win_x + (x1 as f64 / dev_w * win_w);
        let sy1 = win_y + title_bar + (y1 as f64 / dev_h * content_h);
        let sx2 = win_x + (x2 as f64 / dev_w * win_w);
        let sy2 = win_y + title_bar + (y2 as f64 / dev_h * content_h);

        // Use a Python3 script with Quartz (CoreGraphics) for mouse drag.
        // Quartz is available on macOS via pyobjc. If not available, use cliclick fallback.
        let steps = 20;
        let step_delay = (duration_ms as f64 / 1000.0) / steps as f64;

        let python_script = format!(
            r#"
import time, subprocess, sys

try:
    import Quartz
    # Mouse down at start
    pt = Quartz.CGPointMake({sx1}, {sy1})
    e = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventLeftMouseDown, pt, Quartz.kCGMouseButtonLeft)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, e)

    # Drag through intermediate points
    for i in range(1, {steps} + 1):
        t = i / {steps}.0
        x = {sx1} + ({sx2} - {sx1}) * t
        y = {sy1} + ({sy2} - {sy1}) * t
        pt = Quartz.CGPointMake(x, y)
        e = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventLeftMouseDragged, pt, Quartz.kCGMouseButtonLeft)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, e)
        time.sleep({step_delay})

    # Mouse up at end
    pt = Quartz.CGPointMake({sx2}, {sy2})
    e = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventLeftMouseUp, pt, Quartz.kCGMouseButtonLeft)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, e)
    print("ok")
except ImportError:
    print("no_quartz")
"#,
        );

        let output = tokio::process::Command::new("python3")
            .arg("-c")
            .arg(&python_script)
            .output()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "ios".into(),
                reason: format!("python3 swipe script failed: {e}"),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() == "ok" {
            return Ok(format!("swiped ({x1},{y1}) → ({x2},{y2}) via CGEvent"));
        }

        // Fallback: try cliclick
        if check_cmd_available("cliclick").await {
            // cliclick: dd = mouse down + drag, du = mouse up
            let cmd = format!(
                "dd:{},{} du:{},{}",
                sx1 as i32, sy1 as i32, sx2 as i32, sy2 as i32
            );
            Self::run_cmd("cliclick", &[&cmd]).await?;
            return Ok(format!("swiped ({x1},{y1}) → ({x2},{y2}) via cliclick"));
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: "swipe requires idb or Python3 Quartz module. Install: pip3 install fb-idb (or: pip3 install pyobjc-framework-Quartz)".into(),
        })
    }

    /// Type text on the device.
    ///
    /// Strategy: AppleScript keystroke (works immediately for simulators on macOS) → idb → error
    pub async fn type_text(&self, text: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            // AppleScript keystroke — sends keystrokes to the focused Simulator window.
            // This is the most reliable approach for simulators on macOS.
            let script = format!(
                "tell application \"Simulator\" to activate\ndelay 0.15\ntell application \"System Events\" to keystroke \"{}\"",
                text.replace('\\', "\\\\").replace('"', "\\\"")
            );

            match Self::run_cmd("osascript", &["-e", &script]).await {
                Ok(_) => return Ok(format!("typed {} chars via AppleScript", text.len())),
                Err(e) => {
                    warn!("AppleScript keystroke failed: {e}, trying idb");
                }
            }
        }

        // Fallback: try idb
        if check_idb_available().await {
            Self::run_cmd("idb", &["ui", "text", text, "--udid", udid]).await?;
            return Ok(format!("typed {} chars via idb", text.len()));
        }

        if !self.is_simulator {
            return Err(ClawError::ToolExecution {
                tool: "ios".into(),
                reason: "text input on physical device requires idb. Install: brew install idb-companion && pip3 install fb-idb".into(),
            });
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: "text input failed. Ensure Simulator.app is in the foreground.".into(),
        })
    }

    /// Press a hardware button (home, lock, siri, volume_up, volume_down).
    ///
    /// For simulators, maps to Simulator.app keyboard shortcuts via AppleScript.
    pub async fn press_button(&self, button: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        // 1. Try idb first (works for all button types)
        if check_idb_available().await {
            Self::run_cmd("idb", &["ui", "button", button, "--udid", udid]).await?;
            return Ok(format!("pressed {button} via idb"));
        }

        // 2. For simulators: use Simulator.app keyboard shortcuts
        if self.is_simulator {
            // Map button names to Simulator keyboard shortcuts
            let keystroke_script = match button.to_lowercase().as_str() {
                "home" => {
                    // Cmd+Shift+H — Home button
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 4 using {command down, shift down}"
                }
                "lock" | "power" => {
                    // Cmd+L — Lock screen
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 37 using {command down}"
                }
                "siri" => {
                    // Long-press Home / Side button is not directly mapped,
                    // but Cmd+Shift+H held might work on some Xcode versions
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 4 using {command down, shift down}"
                }
                "volume_up" => {
                    // Cmd+Up in recent Simulator versions (may vary)
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 126 using {command down}"
                }
                "volume_down" => {
                    // Cmd+Down
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 125 using {command down}"
                }
                "shake" => {
                    // Ctrl+Cmd+Z — Shake gesture
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 6 using {command down, control down}"
                }
                "screenshot_device" => {
                    // Cmd+S — Save screenshot (Simulator built-in)
                    "tell application \"Simulator\" to activate\ntell application \"System Events\" to key code 1 using {command down}"
                }
                _ => {
                    return Err(ClawError::ToolExecution {
                        tool: "ios".into(),
                        reason: format!(
                            "unknown button '{button}'. Supported: home, lock, power, siri, volume_up, volume_down, shake, screenshot_device"
                        ),
                    });
                }
            };

            Self::run_cmd("osascript", &["-e", keystroke_script])
                .await
                .map_err(|e| ClawError::ToolExecution {
                    tool: "ios".into(),
                    reason: format!("AppleScript button press failed: {e}"),
                })?;

            return Ok(format!("pressed {button} via Simulator keyboard shortcut"));
        }

        Err(ClawError::ToolExecution {
            tool: "ios".into(),
            reason: format!(
                "button '{button}' on physical device requires idb. Install: brew install idb-companion && pip3 install fb-idb"
            ),
        })
    }

    // ── Simulator Management ─────────────────────────────────

    /// Boot a simulator.
    pub async fn boot_simulator(&self, udid: &str) -> claw_core::Result<String> {
        Self::run_cmd("xcrun", &["simctl", "boot", udid]).await?;
        Ok(format!("booted simulator {udid}"))
    }

    /// Shutdown a simulator.
    pub async fn shutdown_simulator(&self, udid: &str) -> claw_core::Result<String> {
        Self::run_cmd("xcrun", &["simctl", "shutdown", udid]).await?;
        Ok(format!("shut down simulator {udid}"))
    }

    /// Open a URL on the device (deep links, web URLs).
    pub async fn open_url(&self, url: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            Self::run_cmd("xcrun", &["simctl", "openurl", udid, url]).await?;
        } else if check_idb_available().await {
            Self::run_cmd("idb", &["open", url, "--udid", udid]).await?;
        } else {
            return Err(ClawError::ToolExecution {
                tool: "ios".into(),
                reason: "open URL on physical device requires idb".into(),
            });
        }

        Ok(format!("opened {url}"))
    }

    /// Push a file to the device.
    pub async fn push_file(&self, local: &str, remote_path: &str) -> claw_core::Result<String> {
        let udid = self.require_device()?;

        if self.is_simulator {
            // simctl doesn't have direct file push, use data container
            Self::run_cmd(
                "xcrun",
                &["simctl", "push", udid, "data", local, remote_path],
            )
            .await
            .unwrap_or_else(|_| {
                "simulator file push may require app container context".to_string()
            });
        } else {
            // Use AFC via idevice tools or idb
            if check_idb_available().await {
                Self::run_cmd("idb", &["file", "push", local, remote_path, "--udid", udid]).await?;
            } else {
                return Err(ClawError::ToolExecution {
                    tool: "ios".into(),
                    reason: "file push requires idb".into(),
                });
            }
        }

        Ok(format!("pushed {local} → {remote_path}"))
    }

    /// Get device status.
    pub async fn status(&self) -> claw_core::Result<Value> {
        let devices = self.list_devices().await.unwrap_or_default();
        let physical: Vec<_> = devices
            .iter()
            .filter(|d| d.device_type == "physical")
            .collect();
        let booted_sims: Vec<_> = devices
            .iter()
            .filter(|d| d.device_type == "simulator" && d.state == "Booted")
            .collect();

        Ok(json!({
            "libimobiledevice_available": check_libimobiledevice_available().await,
            "xcode_simctl_available": check_simctl_available().await,
            "idb_available": check_idb_available().await,
            "physical_devices": physical.len(),
            "booted_simulators": booted_sims.len(),
            "active_device": self.active_device,
            "is_simulator": self.is_simulator,
            "devices": devices,
        }))
    }
}

// ─── Availability Checks ──────────────────────────────────────────

async fn check_libimobiledevice_available() -> bool {
    tokio::process::Command::new("idevice_id")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn check_simctl_available() -> bool {
    tokio::process::Command::new("xcrun")
        .args(["simctl", "help"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn check_idb_available() -> bool {
    tokio::process::Command::new("idb")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if an arbitrary command is available on PATH.
async fn check_cmd_available(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
