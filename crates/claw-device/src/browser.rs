//! Browser automation via Chrome DevTools Protocol (CDP).
//!
//! Manages headless (or headed) Chrome/Chromium instances and communicates
//! over the CDP WebSocket to navigate, click, type, screenshot, and evaluate
//! JavaScript.
//!
//! # Architecture
//!
//! ```text
//!   DeviceTools (tool layer)
//!       │
//!       ▼
//!   BrowserManager          ← Singleton, manages browser lifecycle
//!       │
//!       ├── BrowserInstance ← One per session (tab pool)
//!       │       ├── Tab 0
//!       │       ├── Tab 1
//!       │       └── ...
//!       │
//!       └── CdpClient      ← Low-level CDP JSON-RPC over WebSocket
//! ```

use claw_core::ClawError;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

// ─── Types ──────────────────────────────────────────────────────

/// Information about a browser tab/page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
}

/// A screenshot captured from a browser tab.
#[derive(Debug, Clone)]
pub struct Screenshot {
    /// Base64-encoded PNG image data.
    pub data_base64: String,
    pub width: u32,
    pub height: u32,
}

/// Result of evaluating JavaScript in a tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub value: Value,
    pub is_error: bool,
}

/// Accessibility / DOM snapshot for the agent's "vision".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub url: String,
    pub title: String,
    /// Simplified text representation of the page.
    pub text_content: String,
    /// Clickable elements with their selectors.
    pub interactive_elements: Vec<InteractiveElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveElement {
    pub index: usize,
    pub tag: String,
    pub role: String,
    pub text: String,
    pub selector: String,
}

// ─── CDP Client ──────────────────────────────────────────────────

/// Low-level Chrome DevTools Protocol client.
///
/// Communicates with Chrome via the `/json` HTTP endpoints and sends
/// CDP commands over the WebSocket debugger URL.
struct CdpClient {
    /// Base HTTP URL for the Chrome DevTools API (e.g. http://127.0.0.1:9222).
    base_url: String,
    http: reqwest::Client,
}

impl CdpClient {
    fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            http: reqwest::Client::new(),
        }
    }

    /// List all open tabs.
    async fn list_tabs(&self) -> claw_core::Result<Vec<TabInfo>> {
        let url = format!("{}/json/list", self.base_url);
        let resp: Vec<Value> = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP list tabs failed: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP parse tabs failed: {e}"),
            })?;

        Ok(resp
            .iter()
            .filter_map(|t| {
                if t["type"].as_str() == Some("page") {
                    Some(TabInfo {
                        id: t["id"].as_str().unwrap_or("").to_string(),
                        url: t["url"].as_str().unwrap_or("").to_string(),
                        title: t["title"].as_str().unwrap_or("").to_string(),
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    /// Open a new tab and return its info.
    async fn new_tab(&self, url: &str) -> claw_core::Result<TabInfo> {
        let api_url = format!("{}/json/new?{}", self.base_url, url);
        let resp: Value = self
            .http
            .get(&api_url)
            .send()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP new tab failed: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP parse new tab failed: {e}"),
            })?;

        Ok(TabInfo {
            id: resp["id"].as_str().unwrap_or("").to_string(),
            url: resp["url"].as_str().unwrap_or("").to_string(),
            title: resp["title"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Close a tab by its ID.
    async fn close_tab(&self, tab_id: &str) -> claw_core::Result<()> {
        let url = format!("{}/json/close/{}", self.base_url, tab_id);
        self.http
            .get(&url)
            .send()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP close tab failed: {e}"),
            })?;
        Ok(())
    }

    /// Send a CDP command to a specific tab and return the result.
    /// Uses the `/json/protocol` HTTP endpoint for simple commands,
    /// falling back to the shell-based `chrome-remote-interface` for complex ones.
    async fn send_command(
        &self,
        tab_id: &str,
        method: &str,
        params: Value,
    ) -> claw_core::Result<Value> {
        // We use a lightweight approach: pipe CDP commands through a small
        // node.js/curl helper. For production, this would use a proper
        // WebSocket connection, but the HTTP+curl approach works universally.
        let ws_url = self.get_ws_url(tab_id).await?;

        // Build the CDP JSON-RPC message
        let msg = json!({
            "id": 1,
            "method": method,
            "params": params,
        });

        // Use websocat if available, otherwise fall back to curl-based approach
        let result = self.send_ws_command(&ws_url, &msg).await?;
        Ok(result)
    }

    /// Get the WebSocket debugger URL for a tab.
    async fn get_ws_url(&self, tab_id: &str) -> claw_core::Result<String> {
        let url = format!("{}/json/list", self.base_url);
        let tabs: Vec<Value> = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP get WS URL failed: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("CDP parse WS URL failed: {e}"),
            })?;

        for tab in &tabs {
            if tab["id"].as_str() == Some(tab_id)
                && let Some(ws) = tab["webSocketDebuggerUrl"].as_str()
            {
                return Ok(ws.to_string());
            }
        }

        Err(ClawError::ToolExecution {
            tool: "browser".into(),
            reason: format!("tab {tab_id} not found or no WS URL"),
        })
    }

    /// Send a WebSocket command using native tokio-tungstenite (reliable, no subprocess).
    async fn send_ws_command(&self, ws_url: &str, message: &Value) -> claw_core::Result<Value> {
        let msg_str = serde_json::to_string(message).unwrap();
        let expected_id = message["id"].as_i64().unwrap_or(1);

        // Connect to the Chrome DevTools WebSocket
        let (mut ws, _) = connect_async(ws_url)
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("WebSocket connect failed: {e}"),
            })?;

        // Send the CDP command
        ws.send(Message::Text(msg_str.into()))
            .await
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("WebSocket send failed: {e}"),
            })?;

        // Read responses until we get one matching our command ID.
        // Chrome may send event notifications before our response.
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(resp) = serde_json::from_str::<Value>(&text)
                            && resp.get("id").and_then(|v| v.as_i64()) == Some(expected_id)
                        {
                            return Ok(resp);
                        }
                        // else: this is an event notification, skip it
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        return Err(ClawError::ToolExecution {
                            tool: "browser".into(),
                            reason: format!("WebSocket read error: {e}"),
                        });
                    }
                }
            }
            Err(ClawError::ToolExecution {
                tool: "browser".into(),
                reason: "WebSocket closed before response received".into(),
            })
        })
        .await;

        // Close the connection gracefully
        let _ = ws.close(None).await;

        match result {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ClawError::ToolExecution {
                tool: "browser".into(),
                reason: "CDP command timed out (30s)".into(),
            }),
        }
    }
}

// ─── Browser Instance ────────────────────────────────────────────

/// A running browser instance with its CDP port and process handle.
struct BrowserInstance {
    /// CDP debugging port.
    port: u16,
    /// Process handle (so we can kill it on cleanup).
    process: Option<tokio::process::Child>,
    /// CDP client for this instance.
    cdp: CdpClient,
    /// Whether this was an externally-connected browser (don't kill on drop).
    _external: bool,
}

impl BrowserInstance {
    /// Launch a new headless Chrome/Chromium instance.
    async fn launch(headless: bool, port: u16) -> claw_core::Result<Self> {
        // Find Chrome/Chromium binary
        let chrome_bin = find_chrome_binary()?;

        info!(binary = %chrome_bin, port = port, headless = headless, "launching browser");

        let mut cmd = tokio::process::Command::new(&chrome_bin);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-translate")
            .arg("--metrics-recording-only")
            .arg("--safebrowsing-disable-auto-update")
            .arg("--window-size=1920,1080")
            .arg(format!("--user-data-dir=/tmp/claw-chrome-{port}"));

        if headless {
            cmd.arg("--headless=new");
        }

        // Start with a blank page
        cmd.arg("about:blank");

        let process = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("failed to launch Chrome at '{chrome_bin}': {e}"),
            })?;

        // Wait for Chrome to be ready
        let cdp = CdpClient::new(port);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_secs(10) {
                return Err(ClawError::ToolExecution {
                    tool: "browser".into(),
                    reason: "Chrome failed to start within 10 seconds".into(),
                });
            }
            match cdp.list_tabs().await {
                Ok(_) => break,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
            }
        }

        info!(port = port, "browser ready");

        Ok(Self {
            port,
            process: Some(process),
            cdp,
            _external: false,
        })
    }

    /// Connect to an already-running Chrome instance.
    async fn connect(port: u16) -> claw_core::Result<Self> {
        let cdp = CdpClient::new(port);
        // Verify connection
        cdp.list_tabs().await?;
        info!(port = port, "connected to existing browser");

        Ok(Self {
            port,
            process: None,
            cdp,
            _external: true,
        })
    }

    /// Navigate a tab to a URL and wait for the page to load.
    async fn navigate(&self, tab_id: &str, url: &str) -> claw_core::Result<()> {
        self.cdp
            .send_command(
                tab_id,
                "Page.navigate",
                json!({
                    "url": url,
                }),
            )
            .await?;

        // Wait for load event
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        Ok(())
    }

    /// Take a screenshot of a tab (returns base64 PNG).
    async fn screenshot(&self, tab_id: &str) -> claw_core::Result<Screenshot> {
        let result = self
            .cdp
            .send_command(
                tab_id,
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                }),
            )
            .await?;

        let data = result["result"]["data"].as_str().unwrap_or("").to_string();

        if data.is_empty() {
            return Err(ClawError::ToolExecution {
                tool: "browser".into(),
                reason: "screenshot returned empty data — CDP response may have failed".into(),
            });
        }

        // Get viewport dimensions from the browser
        let metrics = self
            .cdp
            .send_command(tab_id, "Page.getLayoutMetrics", json!({}))
            .await
            .ok();
        let (width, height) = metrics
            .as_ref()
            .map(|m| {
                let w = m["result"]["cssVisualViewport"]["clientWidth"]
                    .as_f64()
                    .unwrap_or(1920.0) as u32;
                let h = m["result"]["cssVisualViewport"]["clientHeight"]
                    .as_f64()
                    .unwrap_or(1080.0) as u32;
                (w, h)
            })
            .unwrap_or((1920, 1080));

        Ok(Screenshot {
            data_base64: data,
            width,
            height,
        })
    }

    /// Evaluate JavaScript in a tab.
    async fn evaluate(&self, tab_id: &str, expression: &str) -> claw_core::Result<EvalResult> {
        let result = self
            .cdp
            .send_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        let value = result["result"]["result"]["value"].clone();
        let is_error = result["result"]["exceptionDetails"].is_object();

        Ok(EvalResult { value, is_error })
    }

    /// Click at coordinates or a CSS selector.
    async fn click(&self, tab_id: &str, selector: &str) -> claw_core::Result<()> {
        // Support two modes:
        // 1. Standard CSS selector (e.g. '#submit-btn', 'button:nth-of-type(2)')
        // 2. Text-based selector with /* text: ... */ hint from snapshot
        //    (e.g. "button:nth-of-type(1) /* text: Transfer */")
        //
        // For text-based hints, we extract the text and find the element by
        // matching both the CSS part and the text content.
        let js = format!(
            r#"(() => {{
                const fullSelector = {sel};
                // Check for text hint: "selector /* text: ... */"
                const textMatch = fullSelector.match(/^(.+?)\s*\/\*\s*text:\s*(.+?)\s*\*\/$/);
                let el = null;

                if (textMatch) {{
                    // Try CSS part first, then filter by text
                    const cssPart = textMatch[1].trim();
                    const textHint = textMatch[2].trim().toLowerCase();
                    try {{
                        const candidates = document.querySelectorAll(cssPart);
                        for (const c of candidates) {{
                            const t = (c.textContent || '').trim().toLowerCase();
                            if (t === textHint || t.includes(textHint)) {{ el = c; break; }}
                        }}
                    }} catch(e) {{}}

                    // If CSS+text didn't work, search all interactive elements by text
                    if (!el) {{
                        const all = document.querySelectorAll('a, button, [role="button"], input[type="submit"]');
                        for (const c of all) {{
                            const t = (c.textContent || c.value || '').trim().toLowerCase();
                            if (t === textHint || t.includes(textHint)) {{ el = c; break; }}
                        }}
                    }}
                }}

                if (!el) {{
                    // Standard CSS selector (also handles text-contains via :has if needed)
                    try {{ el = document.querySelector(fullSelector); }} catch(e) {{}}
                }}

                if (!el) return JSON.stringify({{ error: 'element not found: ' + fullSelector }});
                el.scrollIntoView({{ block: 'center' }});
                el.click();
                return JSON.stringify({{ ok: true, tag: el.tagName, text: el.textContent.slice(0, 100) }});
            }})()"#,
            sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{selector}\"")),
        );
        self.evaluate(tab_id, &js).await?;
        Ok(())
    }

    /// Type text into a focused element or a CSS selector.
    async fn type_text(&self, tab_id: &str, selector: &str, text: &str) -> claw_core::Result<()> {
        // Focus the element first
        let focus_js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return JSON.stringify({{ error: 'element not found' }});
                el.focus();
                return JSON.stringify({{ ok: true }});
            }})()"#,
            selector.replace('\'', "\\'").replace('\\', "\\\\"),
        );
        self.evaluate(tab_id, &focus_js).await?;

        // Then dispatch keyboard events character by character
        for ch in text.chars() {
            self.cdp
                .send_command(
                    tab_id,
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "keyDown",
                        "text": ch.to_string(),
                    }),
                )
                .await?;
            self.cdp
                .send_command(
                    tab_id,
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "keyUp",
                        "text": ch.to_string(),
                    }),
                )
                .await?;
        }
        Ok(())
    }

    /// Get a text snapshot of the page (for the agent to "read").
    async fn snapshot(&self, tab_id: &str) -> claw_core::Result<PageSnapshot> {
        let js = r#"(() => {
            const elements = [];
            const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [onclick], [tabindex]';
            document.querySelectorAll(interactiveSelectors).forEach((el, i) => {
                const rect = el.getBoundingClientRect();
                if (rect.width === 0 && rect.height === 0) return;
                const text = (el.textContent || el.value || el.placeholder || el.alt || el.title || '').trim().slice(0, 200);
                if (!text && el.tagName !== 'INPUT') return;

                // Build a unique, reliable selector — prefer id, then name,
                // then generate a text-based XPath-style selector that the
                // click tool can use to disambiguate buttons with the same
                // structural position.
                let selector = '';
                if (el.id) {
                    selector = '#' + CSS.escape(el.id);
                } else if (el.name) {
                    selector = el.tagName.toLowerCase() + '[name="' + el.name + '"]';
                } else {
                    // Build nth-of-type within parent as baseline
                    const tag = el.tagName.toLowerCase();
                    const siblings = Array.from(el.parentElement?.children || []).filter(c => c.tagName === el.tagName);
                    const idx = siblings.indexOf(el) + 1;
                    const base = tag + ':nth-of-type(' + idx + ')';

                    // If there's only one element with this exact text on the page,
                    // annotate with data-claw-text so click can use it.
                    // But also provide CSS selector that's unambiguous.
                    if (el.parentElement) {
                        // Walk up to find a parent with an id for a more unique path
                        let ancestor = el.parentElement;
                        let path = base;
                        for (let depth = 0; depth < 5 && ancestor && ancestor !== document.body; depth++) {
                            const atag = ancestor.tagName.toLowerCase();
                            if (ancestor.id) {
                                path = '#' + CSS.escape(ancestor.id) + ' > ' + path;
                                break;
                            }
                            const asibs = Array.from(ancestor.parentElement?.children || []).filter(c => c.tagName === ancestor.tagName);
                            const aidx = asibs.indexOf(ancestor) + 1;
                            path = atag + ':nth-of-type(' + aidx + ') > ' + path;
                            ancestor = ancestor.parentElement;
                        }
                        selector = path;
                    } else {
                        selector = base;
                    }
                }

                // Verify uniqueness — if the selector matches multiple elements, try to refine
                try {
                    const matches = document.querySelectorAll(selector);
                    if (matches.length > 1) {
                        // Add aria-label or text-based refinement hint as a comment
                        const label = el.getAttribute('aria-label') || text.slice(0, 50);
                        selector = selector + ' /* text: ' + label.replace(/\*/g, '') + ' */';
                    }
                } catch(e) {}

                elements.push({
                    index: i,
                    tag: el.tagName.toLowerCase(),
                    role: el.getAttribute('role') || el.type || el.tagName.toLowerCase(),
                    text: text,
                    selector: selector,
                });
            });

            // Get readable text content
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
            let textContent = '';
            while (walker.nextNode()) {
                const text = walker.currentNode.textContent.trim();
                if (text.length > 2) textContent += text + '\n';
            }

            return JSON.stringify({
                url: location.href,
                title: document.title,
                text_content: textContent.slice(0, 20000),
                interactive_elements: elements.slice(0, 100),
            });
        })()"#;

        let result = self.evaluate(tab_id, js).await?;
        let text = result.value.as_str().unwrap_or("{}");
        let snapshot: PageSnapshot = serde_json::from_str(text).unwrap_or(PageSnapshot {
            url: String::new(),
            title: String::new(),
            text_content: "failed to capture page snapshot".into(),
            interactive_elements: vec![],
        });

        Ok(snapshot)
    }

    /// Get the page as PDF (base64).
    async fn print_pdf(&self, tab_id: &str) -> claw_core::Result<String> {
        let result = self
            .cdp
            .send_command(
                tab_id,
                "Page.printToPDF",
                json!({
                    "printBackground": true,
                }),
            )
            .await?;

        Ok(result["result"]["data"].as_str().unwrap_or("").to_string())
    }

    /// Scroll the page.
    async fn scroll(&self, tab_id: &str, direction: &str, amount: i32) -> claw_core::Result<()> {
        let (dx, dy) = match direction {
            "down" => (0, amount),
            "up" => (0, -amount),
            "left" => (-amount, 0),
            "right" => (amount, 0),
            _ => (0, amount),
        };

        self.cdp
            .send_command(
                tab_id,
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseWheel",
                    "x": 640,
                    "y": 360,
                    "deltaX": dx,
                    "deltaY": dy,
                }),
            )
            .await?;

        Ok(())
    }

    /// Set files on a `<input type="file">` element via CDP `DOM.setFileInputFiles`.
    ///
    /// Uses `Runtime.evaluate` to resolve the CSS selector to a JS object reference,
    /// then passes the `objectId` to CDP. This is far more reliable than the
    /// `DOM.getDocument` → `DOM.querySelector` → `nodeId` approach, which breaks on
    /// hidden/clipped inputs, dynamically-modified DOM, and stale DOM agent state.
    /// This is the same technique Puppeteer/Playwright use internally.
    async fn upload_file(
        &self,
        tab_id: &str,
        selector: &str,
        file_paths: &[String],
    ) -> claw_core::Result<()> {
        // 1. Resolve the element via Runtime.evaluate (works for hidden, clipped, any element)
        let js = format!(
            r#"(() => {{
                let el = document.querySelector({sel});
                if (el && el.type === 'file') return el;
                if (el) {{
                    // selector matched a non-file element — look for a file input inside it
                    const inner = el.querySelector('input[type="file"]');
                    if (inner) return inner;
                }}
                // Fallback: find any <input type="file"> on the page
                return document.querySelector('input[type="file"]');
            }})()"#,
            sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{selector}\"")),
        );
        let eval_result = self
            .cdp
            .send_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": js,
                    "returnByValue": false,
                }),
            )
            .await?;

        let object_id = eval_result["result"]["result"]["objectId"]
            .as_str()
            .ok_or_else(|| ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!(
                    "file input not found with selector '{selector}' — also tried 'input[type=\"file\"]'"
                ),
            })?;

        // 2. Set the file paths on the input element using objectId (most reliable)
        self.cdp
            .send_command(
                tab_id,
                "DOM.setFileInputFiles",
                json!({
                    "objectId": object_id,
                    "files": file_paths,
                }),
            )
            .await?;

        // 3. Dispatch events that React/Vue/Svelte/vanilla JS all recognize.
        //    - Native 'change' + 'input' events with bubbling
        //    - For React specifically: trigger via the native setter to update React fiber state
        //    - Also try to trigger the dropzone's onDrop with a synthetic drop event + DataTransfer
        let event_js = format!(
            r#"(() => {{
                const el = document.querySelector({sel}) || document.querySelector('input[type="file"]');
                if (!el) return 'no element';

                // Native setter trick for React — sets value through the native descriptor
                // so React's onChange actually fires
                const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
                    HTMLInputElement.prototype, 'value'
                );

                // Dispatch standard events
                el.dispatchEvent(new Event('input', {{ bubbles: true, cancelable: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true, cancelable: true }}));

                // Also try to synthesize a drop event on the closest dropzone container
                // (parent elements with ondrop, onDragOver, role=presentation, etc.)
                let dropzone = el.closest('[role="presentation"]')
                    || el.closest('[class*="dropzone"]')
                    || el.closest('[class*="drop-zone"]')
                    || el.parentElement;
                if (dropzone) {{
                    try {{
                        // Create a DataTransfer with the files from the input
                        const dt = new DataTransfer();
                        for (const f of el.files) dt.items.add(f);
                        const dropEvent = new DragEvent('drop', {{
                            bubbles: true,
                            cancelable: true,
                            dataTransfer: dt,
                        }});
                        dropzone.dispatchEvent(new DragEvent('dragenter', {{ bubbles: true, dataTransfer: dt }}));
                        dropzone.dispatchEvent(new DragEvent('dragover', {{ bubbles: true, dataTransfer: dt }}));
                        dropzone.dispatchEvent(dropEvent);
                    }} catch(e) {{}}
                }}

                return 'dispatched: files=' + el.files.length;
            }})()"#,
            sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{selector}\"")),
        );
        let ev_result = self
            .cdp
            .send_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": event_js,
                    "returnByValue": true,
                }),
            )
            .await;
        info!(result = ?ev_result, "upload event dispatch");

        Ok(())
    }

    async fn shutdown(&mut self) {
        if let Some(ref mut proc) = self.process {
            let _ = proc.kill().await;
            info!(port = self.port, "browser process killed");
        }
        // Clean up temp profile
        let _ = tokio::fs::remove_dir_all(format!("/tmp/claw-chrome-{}", self.port)).await;
    }
}

// ─── Browser Manager ──────────────────────────────────────────────

/// Top-level manager — holds the active browser instance(s).
pub struct BrowserManager {
    instance: Option<BrowserInstance>,
    /// The active tab ID (most recently used).
    active_tab: Option<String>,
    /// Default CDP port.
    default_port: u16,
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            instance: None,
            active_tab: None,
            default_port: 9222,
        }
    }

    /// Ensure a browser is running — launch or connect.
    async fn ensure_browser(&mut self) -> claw_core::Result<&BrowserInstance> {
        if self.instance.is_none() {
            // Try to connect to an existing one first
            match BrowserInstance::connect(self.default_port).await {
                Ok(inst) => {
                    self.instance = Some(inst);
                }
                Err(_) => {
                    // Launch a new one
                    let inst = BrowserInstance::launch(true, self.default_port).await?;
                    self.instance = Some(inst);
                }
            }
        }
        Ok(self.instance.as_ref().unwrap())
    }

    /// Get the active tab ID, creating one if needed.
    async fn ensure_tab(&mut self) -> claw_core::Result<String> {
        self.ensure_browser().await?;
        let browser = self.instance.as_ref().unwrap();

        if let Some(ref id) = self.active_tab {
            // Verify it still exists
            let tabs = browser.cdp.list_tabs().await?;
            if tabs.iter().any(|t| &t.id == id) {
                return Ok(id.clone());
            }
        }

        // Get first tab or create one
        let tabs = browser.cdp.list_tabs().await?;
        if let Some(tab) = tabs.first() {
            self.active_tab = Some(tab.id.clone());
            Ok(tab.id.clone())
        } else {
            let tab = browser.cdp.new_tab("about:blank").await?;
            self.active_tab = Some(tab.id.clone());
            Ok(tab.id)
        }
    }

    // ── Public API called by DeviceTools ──────────────────────

    /// Start a browser (or connect to existing) and return status.
    pub async fn start(&mut self, headless: bool) -> claw_core::Result<String> {
        if self.instance.is_some() {
            return Ok("browser already running".into());
        }

        match BrowserInstance::connect(self.default_port).await {
            Ok(inst) => {
                self.instance = Some(inst);
                Ok(format!(
                    "connected to existing browser on port {}",
                    self.default_port
                ))
            }
            Err(_) => {
                let inst = BrowserInstance::launch(headless, self.default_port).await?;
                self.instance = Some(inst);
                Ok(format!(
                    "launched headless browser on port {}",
                    self.default_port
                ))
            }
        }
    }

    /// Stop the browser.
    pub async fn stop(&mut self) -> claw_core::Result<String> {
        if let Some(ref mut inst) = self.instance {
            inst.shutdown().await;
            self.instance = None;
            self.active_tab = None;
            Ok("browser stopped".into())
        } else {
            Ok("no browser running".into())
        }
    }

    /// Navigate to a URL.
    pub async fn navigate(&mut self, url: &str) -> claw_core::Result<PageSnapshot> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.navigate(&tab_id, url).await?;
        browser.snapshot(&tab_id).await
    }

    /// Take a screenshot.
    pub async fn screenshot(&mut self) -> claw_core::Result<Screenshot> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.screenshot(&tab_id).await
    }

    /// Click on an element by CSS selector.
    pub async fn click(&mut self, selector: &str) -> claw_core::Result<String> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.click(&tab_id, selector).await?;
        // Brief wait for any navigation / JS
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(format!("clicked '{selector}'"))
    }

    /// Type text into an element.
    pub async fn type_text(&mut self, selector: &str, text: &str) -> claw_core::Result<String> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.type_text(&tab_id, selector, text).await?;
        Ok(format!("typed {} chars into '{}'", text.len(), selector))
    }

    /// Upload file(s) to a <input type="file"> element by CSS selector.
    /// Uses CDP DOM.setFileInputFiles — files must be absolute paths accessible to Chrome.
    pub async fn upload_file(
        &mut self,
        selector: &str,
        file_paths: &[String],
    ) -> claw_core::Result<String> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.upload_file(&tab_id, selector, file_paths).await?;
        // Brief wait for any JS event handlers (e.g. preview rendering)
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let count = file_paths.len();
        Ok(format!("uploaded {count} file(s) to '{selector}'"))
    }

    /// Evaluate JavaScript and return the result.
    pub async fn evaluate(&mut self, expression: &str) -> claw_core::Result<EvalResult> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.evaluate(&tab_id, expression).await
    }

    /// Get a text snapshot of the current page.
    pub async fn snapshot(&mut self) -> claw_core::Result<PageSnapshot> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.snapshot(&tab_id).await
    }

    /// List all open tabs.
    pub async fn tabs(&mut self) -> claw_core::Result<Vec<TabInfo>> {
        self.ensure_browser().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.cdp.list_tabs().await
    }

    /// Open a new tab.
    pub async fn new_tab(&mut self, url: &str) -> claw_core::Result<TabInfo> {
        self.ensure_browser().await?;
        let browser = self.instance.as_ref().unwrap();
        let tab = browser.cdp.new_tab(url).await?;
        self.active_tab = Some(tab.id.clone());
        Ok(tab)
    }

    /// Close a tab.
    pub async fn close_tab(&mut self, tab_id: &str) -> claw_core::Result<()> {
        self.ensure_browser().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.cdp.close_tab(tab_id).await?;
        if self.active_tab.as_deref() == Some(tab_id) {
            self.active_tab = None;
        }
        Ok(())
    }

    /// Focus (switch to) a tab.
    pub async fn focus_tab(&mut self, tab_id: &str) -> claw_core::Result<()> {
        self.ensure_browser().await?;
        let browser = self.instance.as_ref().unwrap();
        // Activate via CDP
        browser
            .cdp
            .send_command(tab_id, "Page.bringToFront", json!({}))
            .await?;
        self.active_tab = Some(tab_id.to_string());
        Ok(())
    }

    /// Scroll the page.
    pub async fn scroll(&mut self, direction: &str, amount: i32) -> claw_core::Result<()> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.scroll(&tab_id, direction, amount).await
    }

    /// Get the page as PDF.
    pub async fn pdf(&mut self) -> claw_core::Result<String> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();
        browser.print_pdf(&tab_id).await
    }

    /// Whether a browser is currently running.
    pub fn is_running(&self) -> bool {
        self.instance.is_some()
    }

    /// Get info about the current state.
    pub async fn status(&mut self) -> claw_core::Result<Value> {
        if self.instance.is_none() {
            return Ok(json!({
                "running": false,
            }));
        }

        let tabs = self.tabs().await.unwrap_or_default();
        Ok(json!({
            "running": true,
            "port": self.default_port,
            "active_tab": self.active_tab,
            "tab_count": tabs.len(),
            "tabs": tabs,
        }))
    }

    /// Monitor network requests (fetch/XHR) in the browser.
    /// - "start" installs interceptors for fetch() and XMLHttpRequest
    /// - "get" returns all captured requests
    /// - "clear" resets the capture log
    pub async fn network(&mut self, action: &str) -> claw_core::Result<String> {
        let tab_id = self.ensure_tab().await?;
        let browser = self.instance.as_ref().unwrap();

        let js = match action {
            "start" => r#"(() => {
                if (window.__claw_net) return 'network monitoring already active (' + window.__claw_net.length + ' requests captured)';
                window.__claw_net = [];

                // Intercept fetch()
                const origFetch = window.fetch;
                window.fetch = async function(...args) {
                    const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '?';
                    const method = args[1]?.method || 'GET';
                    const entry = { type: 'fetch', method, url, status: null, body: null, time: Date.now() };
                    try {
                        const resp = await origFetch.apply(this, args);
                        entry.status = resp.status;
                        try {
                            const clone = resp.clone();
                            const text = await clone.text();
                            entry.body = text.slice(0, 2000);
                        } catch(e) {}
                        window.__claw_net.push(entry);
                        return resp;
                    } catch(err) {
                        entry.status = 'error';
                        entry.body = err.message;
                        window.__claw_net.push(entry);
                        throw err;
                    }
                };

                // Intercept XMLHttpRequest
                const origOpen = XMLHttpRequest.prototype.open;
                const origSend = XMLHttpRequest.prototype.send;
                XMLHttpRequest.prototype.open = function(method, url, ...rest) {
                    this.__claw_method = method;
                    this.__claw_url = url;
                    return origOpen.call(this, method, url, ...rest);
                };
                XMLHttpRequest.prototype.send = function(body) {
                    this.addEventListener('load', function() {
                        window.__claw_net.push({
                            type: 'xhr',
                            method: this.__claw_method,
                            url: this.__claw_url,
                            status: this.status,
                            body: (this.responseText || '').slice(0, 2000),
                            time: Date.now(),
                        });
                    });
                    this.addEventListener('error', function() {
                        window.__claw_net.push({
                            type: 'xhr',
                            method: this.__claw_method,
                            url: this.__claw_url,
                            status: 'error',
                            body: null,
                            time: Date.now(),
                        });
                    });
                    return origSend.call(this, body);
                };

                return 'network monitoring started';
            })()"#.to_string(),
            "get" => r#"(() => {
                if (!window.__claw_net) return 'network monitoring not started — call with action=start first';
                if (window.__claw_net.length === 0) return 'no requests captured yet';
                return JSON.stringify(window.__claw_net, null, 2);
            })()"#.to_string(),
            "clear" => r#"(() => {
                if (!window.__claw_net) return 'network monitoring not active';
                const count = window.__claw_net.length;
                window.__claw_net = [];
                return 'cleared ' + count + ' captured requests';
            })()"#.to_string(),
            other => return Err(ClawError::ToolExecution {
                tool: "browser".into(),
                reason: format!("unknown network action '{other}' — use 'start', 'get', or 'clear'"),
            }),
        };

        let result = browser.evaluate(&tab_id, &js).await?;
        let text = match &result.value {
            Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
        Ok(text)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

/// Find a Chrome or Chromium binary on the system.
fn find_chrome_binary() -> claw_core::Result<String> {
    let candidates = [
        // macOS
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        // Linux
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        // PATH fallback
        "chrome",
    ];

    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
        // Check PATH
        if let Ok(output) = std::process::Command::new("which").arg(candidate).output()
            && output.status.success()
        {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    Err(ClawError::ToolExecution {
        tool: "browser".into(),
        reason: "Chrome/Chromium not found. Install Chrome or set CHROME_PATH.".into(),
    })
}
