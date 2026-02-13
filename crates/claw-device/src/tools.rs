//! Device tool definitions and executor.
//!
//! Exposes browser, Android, and iOS capabilities as tools that the LLM can call.
//! All tools follow the `claw-core` `ToolExecutor` pattern.

use crate::{BrowserManager, AndroidBridge, IosBridge};
use claw_core::{ClawError, Tool, ToolCall, ToolResult};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Save a base64-encoded PNG screenshot to ~/.claw/screenshots/ and return
/// the relative URL path (e.g. "/api/v1/screenshots/browser_1707654321_abc.png").
async fn save_screenshot(prefix: &str, base64_data: &str) -> claw_core::Result<(String, String)> {
    use base64::Engine;

    let screenshots_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".claw")
        .join("screenshots");

    tokio::fs::create_dir_all(&screenshots_dir).await.map_err(|e| ClawError::ToolExecution {
        tool: "screenshot".into(),
        reason: format!("failed to create screenshots dir: {e}"),
    })?;

    // Generate a unique filename: {prefix}_{timestamp}_{short_rand}.png
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rand_suffix: u32 = rand::random::<u32>() % 100_000;
    let filename = format!("{}_{ts}_{rand_suffix:05}.png", prefix);
    let filepath = screenshots_dir.join(&filename);

    // Decode base64 → raw PNG bytes → write to disk
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| ClawError::ToolExecution {
            tool: "screenshot".into(),
            reason: format!("invalid base64 screenshot data: {e}"),
        })?;

    tokio::fs::write(&filepath, &bytes).await.map_err(|e| ClawError::ToolExecution {
        tool: "screenshot".into(),
        reason: format!("failed to write screenshot: {e}"),
    })?;

    let url_path = format!("/api/v1/screenshots/{filename}");
    let disk_path = filepath.to_string_lossy().to_string();
    Ok((url_path, disk_path))
}

/// Holds managed device backends and dispatches tool calls.
pub struct DeviceTools {
    pub browser: Arc<Mutex<BrowserManager>>,
    pub android: Arc<Mutex<AndroidBridge>>,
    pub ios: Arc<Mutex<IosBridge>>,
}

impl DeviceTools {
    pub fn new() -> Self {
        Self {
            browser: Arc::new(Mutex::new(BrowserManager::new())),
            android: Arc::new(Mutex::new(AndroidBridge::new())),
            ios: Arc::new(Mutex::new(IosBridge::new())),
        }
    }

    /// Check if a tool name belongs to the device subsystem.
    pub fn has_tool(name: &str) -> bool {
        matches!(
            name,
            // Browser
            "browser_start"
                | "browser_stop"
                | "browser_navigate"
                | "browser_screenshot"
                | "browser_click"
                | "browser_type"
                | "browser_evaluate"
                | "browser_snapshot"
                | "browser_tabs"
                | "browser_new_tab"
                | "browser_close_tab"
                | "browser_scroll"
                | "browser_upload_file"
                | "browser_network"
                | "browser_status"
                // Android
                | "android_devices"
                | "android_select"
                | "android_screenshot"
                | "android_tap"
                | "android_swipe"
                | "android_type"
                | "android_key"
                | "android_shell"
                | "android_launch"
                | "android_stop_app"
                | "android_install"
                | "android_apps"
                | "android_screen_info"
                | "android_ui_dump"
                | "android_status"
                // iOS
                | "ios_devices"
                | "ios_select"
                | "ios_screenshot"
                | "ios_tap"
                | "ios_swipe"
                | "ios_type"
                | "ios_button"
                | "ios_launch"
                | "ios_terminate"
                | "ios_install"
                | "ios_apps"
                | "ios_open_url"
                | "ios_boot_sim"
                | "ios_shutdown_sim"
                | "ios_status"
        )
    }

    /// Return all device tool definitions for the LLM.
    pub fn tools() -> Vec<Tool> {
        let mut tools = Vec::new();

        // ── Browser Tools ─────────────────────────────────────
        tools.push(Tool {
            name: "browser_start".into(),
            description: "Start a headless Chrome/Chromium browser for web automation. Auto-connects to existing Chrome if one is running with remote debugging.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "headless": {
                        "type": "boolean",
                        "description": "Run headless (no visible window). Default: true"
                    }
                }
            }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_stop".into(),
            description: "Stop the browser and clean up.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_navigate".into(),
            description: "Navigate to a URL. Returns a text snapshot of the page including clickable elements with their CSS selectors. Use this to browse the web.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to navigate to"
                    }
                },
                "required": ["url"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 2,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_screenshot".into(),
            description: "Take a screenshot of the current browser tab. Returns base64 PNG image.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_click".into(),
            description: "Click on an element by CSS selector. Supports text hints from browser_snapshot (e.g. \"button:nth-of-type(1) /* text: Transfer */\") to disambiguate elements with similar selectors. Use browser_snapshot to find available selectors first.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector of the element to click (e.g. '#submit-btn', 'a.nav-link', 'button:nth-of-type(2)')"
                    }
                },
                "required": ["selector"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_type".into(),
            description: "Type text into a form field or element identified by CSS selector.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector of the input element"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type"
                    }
                },
                "required": ["selector", "text"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_evaluate".into(),
            description: "Execute JavaScript in the browser tab and return the result. Useful for extracting data, interacting with page APIs, or performing complex automation.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "JavaScript expression to evaluate. Use an IIFE for multi-line: (() => { ... })()"
                    }
                },
                "required": ["expression"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 5,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_snapshot".into(),
            description: "Get a text representation of the current page: URL, title, readable text content, and all interactive elements (links, buttons, inputs) with their CSS selectors. Use this to 'see' the page.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_tabs".into(),
            description: "List all open browser tabs.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_new_tab".into(),
            description: "Open a new browser tab with the given URL.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to open in the new tab"
                    }
                },
                "required": ["url"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 2,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_close_tab".into(),
            description: "Close a browser tab by its ID.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "tab_id": {
                        "type": "string",
                        "description": "Tab ID (from browser_tabs)"
                    }
                },
                "required": ["tab_id"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 2,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_scroll".into(),
            description: "Scroll the page in a direction.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "left", "right"],
                        "description": "Scroll direction"
                    },
                    "amount": {
                        "type": "integer",
                        "description": "Scroll amount in pixels. Default: 500"
                    }
                },
                "required": ["direction"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_upload_file".into(),
            description: "Upload file(s) to a file input element on the page. Works with hidden, clipped, and visually-invisible inputs (common in modern upload UIs / dropzones). Pass a CSS selector for the input (e.g. 'input[type=file]') and an array of absolute file paths. Screenshots from browser_screenshot are saved at ~/.claw/screenshots/; use those paths directly, or find files via shell_exec/file_find. If the selector doesn't match, falls back to any input[type=file] on the page.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector of the <input type=\"file\"> element (e.g. 'input[type=file]', '#file-upload')"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Array of absolute file paths to upload (e.g. ['/Users/me/photo.png'])"
                    }
                },
                "required": ["selector", "files"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: true,
            risk_level: 4,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_status".into(),
            description: "Get the current browser status: running, port, active tab, tab count.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "browser_network".into(),
            description: "Monitor network requests (fetch/XHR) in the browser. Call with action='start' to begin capturing, action='get' to retrieve captured requests, action='clear' to reset. Shows method, URL, status, and response body summary for each request. Useful for debugging uploads, API calls, and form submissions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "'start' to begin capturing network requests, 'get' to retrieve captured requests, 'clear' to reset the capture log",
                        "enum": ["start", "get", "clear"]
                    }
                },
                "required": ["action"]
            }),
            capabilities: vec!["browser".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        // ── Android Tools ─────────────────────────────────────
        tools.push(Tool {
            name: "android_devices".into(),
            description: "List connected Android devices via ADB. Shows serial number, model, Android version, and state.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "android_select".into(),
            description: "Select which Android device to target by serial number. Required when multiple devices are connected.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "serial": {
                        "type": "string",
                        "description": "Device serial number from android_devices"
                    }
                },
                "required": ["serial"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "android_screenshot".into(),
            description: "Take a screenshot of the Android device screen. Returns base64 PNG.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "android_tap".into(),
            description: "Tap at screen coordinates on the Android device.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "X coordinate" },
                    "y": { "type": "integer", "description": "Y coordinate" }
                },
                "required": ["x", "y"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "android_swipe".into(),
            description: "Swipe from one point to another on the Android device. Useful for scrolling (swipe up/down) or navigation gestures.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x1": { "type": "integer", "description": "Start X" },
                    "y1": { "type": "integer", "description": "Start Y" },
                    "x2": { "type": "integer", "description": "End X" },
                    "y2": { "type": "integer", "description": "End Y" },
                    "duration_ms": { "type": "integer", "description": "Duration in milliseconds. Default: 300" }
                },
                "required": ["x1", "y1", "x2", "y2"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "android_type".into(),
            description: "Type text on the Android device (into the currently focused input).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to type" }
                },
                "required": ["text"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "android_key".into(),
            description: "Press a key on the Android device: home, back, enter, menu, power, volume_up, volume_down, delete, tab, escape.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key name (home, back, enter, menu, power, etc.)" }
                },
                "required": ["key"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "android_shell".into(),
            description: "Run a shell command on the Android device via ADB. Returns stdout.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run on the device" }
                },
                "required": ["command"]
            }),
            capabilities: vec!["android".into(), "shell".into()],
            is_mutating: true,
            risk_level: 6,
            provider: None,
        });

        tools.push(Tool {
            name: "android_launch".into(),
            description: "Launch an Android app by package name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": { "type": "string", "description": "Package name, e.g. com.android.chrome" }
                },
                "required": ["package"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "android_stop_app".into(),
            description: "Force-stop an Android app by package name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": { "type": "string", "description": "Package name to force-stop" }
                },
                "required": ["package"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 4,
            provider: None,
        });

        tools.push(Tool {
            name: "android_install".into(),
            description: "Install an APK on the Android device.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "apk_path": { "type": "string", "description": "Path to the APK file on the host machine" }
                },
                "required": ["apk_path"]
            }),
            capabilities: vec!["android".into()],
            is_mutating: true,
            risk_level: 5,
            provider: None,
        });

        tools.push(Tool {
            name: "android_apps".into(),
            description: "List third-party apps installed on the Android device.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "android_screen_info".into(),
            description: "Get Android screen info: resolution, density, and current foreground activity/app.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "android_ui_dump".into(),
            description: "Dump the UI hierarchy (accessibility tree) of the Android device as XML. Shows all visible elements, their text, bounds, and resource IDs. Use this to find tap targets.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "android_status".into(),
            description: "Check Android/ADB status: is ADB available, how many devices connected.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["android".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        // ── iOS Tools ─────────────────────────────────────────
        tools.push(Tool {
            name: "ios_devices".into(),
            description: "List connected iOS devices (physical via USB/Wi-Fi) and simulators. Shows UDID, name, model, iOS version.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["ios".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_select".into(),
            description: "Select an iOS device or simulator by UDID. Use ios_devices to find available UDIDs.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "udid": { "type": "string", "description": "Device UDID" },
                    "is_simulator": { "type": "boolean", "description": "Whether this is a simulator. Default: false" }
                },
                "required": ["udid"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_screenshot".into(),
            description: "Take a screenshot of the iOS device/simulator. Returns base64 PNG.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["ios".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_tap".into(),
            description: "Tap at screen coordinates on the iOS device. Requires idb for physical devices.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "X coordinate" },
                    "y": { "type": "integer", "description": "Y coordinate" }
                },
                "required": ["x", "y"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_swipe".into(),
            description: "Swipe on the iOS device screen.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x1": { "type": "integer", "description": "Start X" },
                    "y1": { "type": "integer", "description": "Start Y" },
                    "x2": { "type": "integer", "description": "End X" },
                    "y2": { "type": "integer", "description": "End Y" },
                    "duration_ms": { "type": "integer", "description": "Duration in milliseconds. Default: 300" }
                },
                "required": ["x1", "y1", "x2", "y2"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_type".into(),
            description: "Type text on the iOS device (into the currently focused input).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to type" }
                },
                "required": ["text"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_button".into(),
            description: "Press a hardware button on the iOS device: home, lock, siri, volume_up, volume_down.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "button": { "type": "string", "description": "Button name (home, lock, siri, etc.)" }
                },
                "required": ["button"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_launch".into(),
            description: "Launch an iOS app by bundle identifier.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "bundle_id": { "type": "string", "description": "App bundle identifier, e.g. com.apple.safari" }
                },
                "required": ["bundle_id"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_terminate".into(),
            description: "Terminate an iOS app by bundle identifier.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "bundle_id": { "type": "string", "description": "App bundle identifier" }
                },
                "required": ["bundle_id"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 4,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_install".into(),
            description: "Install an app on the iOS device (.ipa for physical, .app for simulator).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the .ipa or .app file" }
                },
                "required": ["path"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 5,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_apps".into(),
            description: "List installed apps on the iOS device.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["ios".into()],
            is_mutating: false,
            risk_level: 1,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_open_url".into(),
            description: "Open a URL on the iOS device (web URLs or deep links / universal links).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to open" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 3,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_boot_sim".into(),
            description: "Boot an iOS simulator by UDID.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "udid": { "type": "string", "description": "Simulator UDID from ios_devices" }
                },
                "required": ["udid"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 2,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_shutdown_sim".into(),
            description: "Shutdown an iOS simulator.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "udid": { "type": "string", "description": "Simulator UDID" }
                },
                "required": ["udid"]
            }),
            capabilities: vec!["ios".into()],
            is_mutating: true,
            risk_level: 2,
            provider: None,
        });

        tools.push(Tool {
            name: "ios_status".into(),
            description: "Check iOS tooling status: libimobiledevice, simctl, idb availability and connected devices.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            capabilities: vec!["ios".into()],
            is_mutating: false,
            risk_level: 0,
            provider: None,
        });

        tools
    }

    // ── Tool Execution Dispatch ────────────────────────────────

    /// Execute a device tool call.
    pub async fn execute(&self, call: &ToolCall) -> claw_core::Result<ToolResult> {
        match call.tool_name.as_str() {
            // ── Browser ───────────────────────────────────────
            "browser_start" => {
                let headless = call.arguments["headless"].as_bool().unwrap_or(true);
                let mut browser = self.browser.lock().await;
                let msg = browser.start(headless).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "browser_stop" => {
                let mut browser = self.browser.lock().await;
                let msg = browser.stop().await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "browser_navigate" => {
                let url = require_str(call, "url")?;
                let mut browser = self.browser.lock().await;
                let snapshot = browser.navigate(url).await?;
                let content = format_page_snapshot(&snapshot);
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: Some(serde_json::to_value(&snapshot).unwrap_or_default()) })
            }
            "browser_screenshot" => {
                let mut browser = self.browser.lock().await;
                let shot = browser.screenshot().await?;
                let (url_path, disk_path) = save_screenshot("browser", &shot.data_base64).await?;
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Screenshot saved to {} ({}x{}). View: {}", disk_path, shot.width, shot.height, url_path),
                    is_error: false,
                    data: Some(json!({ "screenshot_url": url_path, "screenshot_path": disk_path, "width": shot.width, "height": shot.height })),
                })
            }
            "browser_click" => {
                let selector = require_str(call, "selector")?;
                let mut browser = self.browser.lock().await;
                let msg = browser.click(selector).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "browser_type" => {
                let selector = require_str(call, "selector")?;
                let text = require_str(call, "text")?;
                let mut browser = self.browser.lock().await;
                let msg = browser.type_text(selector, text).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "browser_evaluate" => {
                let expression = require_str(call, "expression")?;
                let mut browser = self.browser.lock().await;
                let result = browser.evaluate(expression).await?;
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: serde_json::to_string_pretty(&result.value).unwrap_or_default(),
                    is_error: result.is_error,
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                })
            }
            "browser_snapshot" => {
                let mut browser = self.browser.lock().await;
                let snapshot = browser.snapshot().await?;
                let content = format_page_snapshot(&snapshot);
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: Some(serde_json::to_value(&snapshot).unwrap_or_default()) })
            }
            "browser_tabs" => {
                let mut browser = self.browser.lock().await;
                let tabs = browser.tabs().await?;
                let content = serde_json::to_string_pretty(&tabs).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "browser_new_tab" => {
                let url = require_str(call, "url")?;
                let mut browser = self.browser.lock().await;
                let tab = browser.new_tab(url).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: serde_json::to_string_pretty(&tab).unwrap_or_default(), is_error: false, data: None })
            }
            "browser_close_tab" => {
                let tab_id = require_str(call, "tab_id")?;
                let mut browser = self.browser.lock().await;
                browser.close_tab(tab_id).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: format!("closed tab {}", tab_id), is_error: false, data: None })
            }
            "browser_scroll" => {
                let direction = require_str(call, "direction")?;
                let amount = call.arguments["amount"].as_i64().unwrap_or(500) as i32;
                let mut browser = self.browser.lock().await;
                browser.scroll(direction, amount).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: format!("scrolled {} {}px", direction, amount), is_error: false, data: None })
            }
            "browser_upload_file" => {
                let selector = require_str(call, "selector")?;
                let files: Vec<String> = call.arguments["files"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                if files.is_empty() {
                    return Ok(ToolResult {
                        tool_call_id: call.id.clone(),
                        content: "Error: 'files' must be a non-empty array of file paths".into(),
                        is_error: true,
                        data: None,
                    });
                }
                // Verify all files exist before attempting upload
                for path in &files {
                    if !std::path::Path::new(path).exists() {
                        return Ok(ToolResult {
                            tool_call_id: call.id.clone(),
                            content: format!("Error: file not found: {}", path),
                            is_error: true,
                            data: None,
                        });
                    }
                }
                let mut browser = self.browser.lock().await;
                let msg = browser.upload_file(selector, &files).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "browser_status" => {
                let mut browser = self.browser.lock().await;
                let status = browser.status().await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: serde_json::to_string_pretty(&status).unwrap_or_default(), is_error: false, data: Some(status) })
            }
            "browser_network" => {
                let action = require_str(call, "action")?;
                let mut browser = self.browser.lock().await;
                let result = browser.network(action).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: result, is_error: false, data: None })
            }

            // ── Android ───────────────────────────────────────
            "android_devices" => {
                let android = self.android.lock().await;
                let devices = android.list_devices().await?;
                let content = serde_json::to_string_pretty(&devices).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "android_select" => {
                let serial = require_str(call, "serial")?;
                let mut android = self.android.lock().await;
                android.select_device(serial);
                Ok(ToolResult { tool_call_id: call.id.clone(), content: format!("selected device {}", serial), is_error: false, data: None })
            }
            "android_screenshot" => {
                let android = self.android.lock().await;
                let shot = android.screenshot().await?;
                let (url_path, disk_path) = save_screenshot("android", &shot.data_base64).await?;
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Screenshot saved to {}. View: {}", disk_path, url_path),
                    is_error: false,
                    data: Some(json!({ "screenshot_url": url_path, "screenshot_path": disk_path })),
                })
            }
            "android_tap" => {
                let x = call.arguments["x"].as_u64().unwrap_or(0) as u32;
                let y = call.arguments["y"].as_u64().unwrap_or(0) as u32;
                let android = self.android.lock().await;
                let msg = android.tap(x, y).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_swipe" => {
                let x1 = call.arguments["x1"].as_u64().unwrap_or(0) as u32;
                let y1 = call.arguments["y1"].as_u64().unwrap_or(0) as u32;
                let x2 = call.arguments["x2"].as_u64().unwrap_or(0) as u32;
                let y2 = call.arguments["y2"].as_u64().unwrap_or(0) as u32;
                let dur = call.arguments["duration_ms"].as_u64().unwrap_or(300) as u32;
                let android = self.android.lock().await;
                let msg = android.swipe(x1, y1, x2, y2, dur).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_type" => {
                let text = require_str(call, "text")?;
                let android = self.android.lock().await;
                let msg = android.type_text(text).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_key" => {
                let key = require_str(call, "key")?;
                let android = self.android.lock().await;
                let msg = android.press_key(key).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_shell" => {
                let command = require_str(call, "command")?;
                let android = self.android.lock().await;
                let output = android.run_shell(command).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: output, is_error: false, data: None })
            }
            "android_launch" => {
                let package = require_str(call, "package")?;
                let android = self.android.lock().await;
                let msg = android.launch_app(package).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_stop_app" => {
                let package = require_str(call, "package")?;
                let android = self.android.lock().await;
                let msg = android.stop_app(package).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_install" => {
                let apk_path = require_str(call, "apk_path")?;
                let android = self.android.lock().await;
                let msg = android.install(apk_path).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "android_apps" => {
                let android = self.android.lock().await;
                let apps = android.list_apps().await?;
                let content = serde_json::to_string_pretty(&apps).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "android_screen_info" => {
                let android = self.android.lock().await;
                let info = android.screen_info().await?;
                let content = serde_json::to_string_pretty(&info).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "android_ui_dump" => {
                let android = self.android.lock().await;
                let xml = android.dump_ui().await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: xml, is_error: false, data: None })
            }
            "android_status" => {
                let android = self.android.lock().await;
                let status = android.status().await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: serde_json::to_string_pretty(&status).unwrap_or_default(), is_error: false, data: Some(status) })
            }

            // ── iOS ───────────────────────────────────────────
            "ios_devices" => {
                let ios = self.ios.lock().await;
                let devices = ios.list_devices().await?;
                let content = serde_json::to_string_pretty(&devices).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "ios_select" => {
                let udid = require_str(call, "udid")?;
                let is_sim = call.arguments["is_simulator"].as_bool().unwrap_or(false);
                let mut ios = self.ios.lock().await;
                ios.select_device(udid, is_sim);
                Ok(ToolResult { tool_call_id: call.id.clone(), content: format!("selected {} {}", if is_sim { "simulator" } else { "device" }, udid), is_error: false, data: None })
            }
            "ios_screenshot" => {
                let ios = self.ios.lock().await;
                let shot = ios.screenshot().await?;
                let (url_path, disk_path) = save_screenshot("ios", &shot.data_base64).await?;
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Screenshot saved to {}. View: {}", disk_path, url_path),
                    is_error: false,
                    data: Some(json!({ "screenshot_url": url_path, "screenshot_path": disk_path })),
                })
            }
            "ios_tap" => {
                let x = call.arguments["x"].as_u64().unwrap_or(0) as u32;
                let y = call.arguments["y"].as_u64().unwrap_or(0) as u32;
                let ios = self.ios.lock().await;
                let msg = ios.tap(x, y).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_swipe" => {
                let x1 = call.arguments["x1"].as_u64().unwrap_or(0) as u32;
                let y1 = call.arguments["y1"].as_u64().unwrap_or(0) as u32;
                let x2 = call.arguments["x2"].as_u64().unwrap_or(0) as u32;
                let y2 = call.arguments["y2"].as_u64().unwrap_or(0) as u32;
                let dur = call.arguments["duration_ms"].as_u64().unwrap_or(300) as u32;
                let ios = self.ios.lock().await;
                let msg = ios.swipe(x1, y1, x2, y2, dur).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_type" => {
                let text = require_str(call, "text")?;
                let ios = self.ios.lock().await;
                let msg = ios.type_text(text).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_button" => {
                let button = require_str(call, "button")?;
                let ios = self.ios.lock().await;
                let msg = ios.press_button(button).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_launch" => {
                let bundle_id = require_str(call, "bundle_id")?;
                let ios = self.ios.lock().await;
                let msg = ios.launch_app(bundle_id).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_terminate" => {
                let bundle_id = require_str(call, "bundle_id")?;
                let ios = self.ios.lock().await;
                let msg = ios.terminate_app(bundle_id).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_install" => {
                let path = require_str(call, "path")?;
                let ios = self.ios.lock().await;
                let msg = ios.install_app(path).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_apps" => {
                let ios = self.ios.lock().await;
                let apps = ios.list_apps().await?;
                let content = serde_json::to_string_pretty(&apps).unwrap_or_default();
                Ok(ToolResult { tool_call_id: call.id.clone(), content, is_error: false, data: None })
            }
            "ios_open_url" => {
                let url = require_str(call, "url")?;
                let ios = self.ios.lock().await;
                let msg = ios.open_url(url).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_boot_sim" => {
                let udid = require_str(call, "udid")?;
                let ios = self.ios.lock().await;
                let msg = ios.boot_simulator(udid).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_shutdown_sim" => {
                let udid = require_str(call, "udid")?;
                let ios = self.ios.lock().await;
                let msg = ios.shutdown_simulator(udid).await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: msg, is_error: false, data: None })
            }
            "ios_status" => {
                let ios = self.ios.lock().await;
                let status = ios.status().await?;
                Ok(ToolResult { tool_call_id: call.id.clone(), content: serde_json::to_string_pretty(&status).unwrap_or_default(), is_error: false, data: Some(status) })
            }

            _ => Err(ClawError::ToolNotFound(call.tool_name.clone())),
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

/// Extract a required string argument from a tool call.
fn require_str<'a>(call: &'a ToolCall, key: &str) -> claw_core::Result<&'a str> {
    call.arguments[key].as_str().ok_or_else(|| ClawError::ToolExecution {
        tool: call.tool_name.clone(),
        reason: format!("missing '{}' argument", key),
    })
}

/// Format a page snapshot into a readable text for the LLM.
fn format_page_snapshot(snapshot: &crate::browser::PageSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {} ({})\n\n", snapshot.title, snapshot.url));

    if !snapshot.text_content.is_empty() {
        let truncated: String = snapshot.text_content.chars().take(5000).collect();
        out.push_str("### Page Content\n");
        out.push_str(&truncated);
        if snapshot.text_content.len() > 5000 {
            out.push_str("\n... (truncated)");
        }
        out.push_str("\n\n");
    }

    if !snapshot.interactive_elements.is_empty() {
        out.push_str("### Interactive Elements\n");
        for el in &snapshot.interactive_elements {
            out.push_str(&format!(
                "[{}] <{}> role={} text=\"{}\" selector=\"{}\"\n",
                el.index, el.tag, el.role, el.text.chars().take(80).collect::<String>(), el.selector
            ));
        }
    }

    out
}
