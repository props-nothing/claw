use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::*;

/// Signal channel adapter using signal-cli's JSON-RPC mode for receiving.
///
/// ## Setup
///
/// Signal requires `signal-cli` to be installed and a phone number to be registered.
///
/// 1. Install signal-cli: `brew install signal-cli` (macOS) or from GitHub releases
/// 2. Register: `signal-cli -u +1234567890 register`
/// 3. Verify: `signal-cli -u +1234567890 verify CODE`
/// 4. Configure in claw.toml:
///    ```toml
///    [channels.signal]
///    type = "signal"
///    token = "+1234567890"
///    ```
pub struct SignalChannel {
    id: String,
    phone: String,
    connected: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl SignalChannel {
    pub fn new(id: String, phone: String) -> Self {
        Self {
            id,
            phone,
            connected: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
        }
    }

    /// Check if signal-cli is available on the system.
    pub fn is_signal_cli_available() -> bool {
        std::process::Command::new("signal-cli")
            .arg("--version")
            .output()
            .is_ok()
    }
}

#[async_trait]
impl Channel for SignalChannel {
    fn id(&self) -> &str {
        &self.id
    }
    fn channel_type(&self) -> &str {
        "signal"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        if !Self::is_signal_cli_available() {
            return Err(claw_core::ClawError::Channel {
                channel: "signal".into(),
                reason: "signal-cli not found. Install it: brew install signal-cli (macOS) \
                         or see https://github.com/AsamK/signal-cli"
                    .into(),
            });
        }

        let phone = self.phone.clone();
        let connected = self.connected.clone();
        let channel_id = self.id.clone();

        tokio::spawn(async move {
            signal_receive_loop(phone, channel_id, event_tx, shutdown_rx, connected).await;
        });

        Ok(event_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        let output = tokio::process::Command::new("signal-cli")
            .args([
                "-u",
                &self.phone,
                "send",
                "-m",
                &message.text,
                &message.target,
            ])
            .output()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "signal".into(),
                reason: format!("signal-cli send failed: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(error = %stderr, "signal-cli send error");
            return Err(claw_core::ClawError::Channel {
                channel: "signal".into(),
                reason: format!("signal-cli send failed: {}", stderr),
            });
        }

        Ok(())
    }

    async fn send_typing(&self, target: &str) -> claw_core::Result<()> {
        // signal-cli doesn't support sending typing indicators in basic mode
        // but we try anyway — the command is available in some versions
        let _ = tokio::process::Command::new("signal-cli")
            .args(["-u", &self.phone, "sendTyping", target])
            .output()
            .await;
        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        self.connected.store(false, Ordering::SeqCst);
        info!("Signal channel stopped");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Receive loop: runs `signal-cli -u PHONE receive --json --timeout 5` in a loop,
/// parsing each JSON line into channel events.
async fn signal_receive_loop(
    phone: String,
    channel_id: String,
    event_tx: mpsc::Sender<ChannelEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    connected: Arc<AtomicBool>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    connected.store(true, Ordering::SeqCst);
    let _ = event_tx.send(ChannelEvent::Connected).await;
    info!(phone = %phone, "Signal receive loop started");

    let mut backoff = 1u64;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        // Spawn signal-cli receive with --json for machine-readable output
        // --timeout 5 means it will block for up to 5 seconds waiting for messages
        let child = tokio::process::Command::new("signal-cli")
            .args(["-u", &phone, "--output=json", "receive", "--timeout", "5"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "Failed to spawn signal-cli receive");
                connected.store(false, Ordering::SeqCst);
                let _ = event_tx
                    .send(ChannelEvent::Disconnected(Some(format!(
                        "signal-cli spawn failed: {}",
                        e
                    ))))
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
        };

        backoff = 1;
        connected.store(true, Ordering::SeqCst);

        // Read stdout line by line
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            let _ = child.kill().await;
                            return;
                        }
                    }
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                if line.trim().is_empty() {
                                    continue;
                                }
                                let parsed: Value = match serde_json::from_str(&line) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        debug!(error = %e, "Signal: could not parse JSON line");
                                        continue;
                                    }
                                };

                                if let Some(incoming) = parse_signal_message(&parsed, &channel_id) {
                                    debug!(
                                        sender = %incoming.sender,
                                        "Signal message received"
                                    );
                                    if event_tx.send(ChannelEvent::Message(incoming)).await.is_err() {
                                        warn!("Signal: event channel closed");
                                        let _ = child.kill().await;
                                        return;
                                    }
                                }
                            }
                            Ok(None) => {
                                // Process exited — stdout closed
                                break;
                            }
                            Err(e) => {
                                warn!(error = %e, "Signal: error reading stdout");
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Wait for process to finish
        let _ = child.wait().await;

        if *shutdown_rx.borrow() {
            break;
        }

        // Short pause before next receive cycle (signal-cli's --timeout handles the main wait)
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    connected.store(false, Ordering::SeqCst);
}

/// Parse a signal-cli JSON envelope into an IncomingMessage.
///
/// signal-cli --output=json emits objects like:
/// ```json
/// {
///   "envelope": {
///     "source": "+1234567890",
///     "sourceName": "Alice",
///     "timestamp": 1234567890000,
///     "dataMessage": {
///       "message": "Hello!",
///       "timestamp": 1234567890000,
///       "groupInfo": { "groupId": "...", "type": "DELIVER" }
///     }
///   }
/// }
/// ```
fn parse_signal_message(payload: &Value, channel_id: &str) -> Option<IncomingMessage> {
    let envelope = payload.get("envelope")?;
    let source = envelope["source"].as_str()?;
    let source_name = envelope["sourceName"].as_str().map(|s| s.to_string());
    let timestamp = envelope["timestamp"].as_u64().unwrap_or(0);

    // Handle data messages (regular text messages)
    let data_msg = envelope.get("dataMessage")?;
    let text = data_msg["message"].as_str();

    if text.is_none() && data_msg.get("attachments").is_none() {
        return None; // No content — probably a read receipt or typing indicator
    }

    // Check for group info
    let group = data_msg
        .get("groupInfo")
        .and_then(|g| g["groupId"].as_str())
        .map(|s| s.to_string());

    // Parse attachments
    let mut attachments = Vec::new();
    if let Some(atts) = data_msg["attachments"].as_array() {
        for att in atts {
            let filename = att["filename"].as_str().unwrap_or("attachment").to_string();
            let content_type = att["contentType"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .to_string();
            // signal-cli saves attachments to a file and provides the path
            let file_path = att["file"].as_str().unwrap_or("").to_string();
            if !file_path.is_empty() {
                attachments.push(Attachment {
                    filename,
                    media_type: content_type,
                    data: file_path,
                });
            }
        }
    }

    Some(IncomingMessage {
        id: format!("signal-{}", timestamp),
        channel: channel_id.to_string(),
        sender: source.to_string(),
        sender_name: source_name,
        group,
        text: text.map(|s| s.to_string()),
        attachments,
        is_mention: false,
        is_reply_to_bot: false,
        metadata: envelope.clone(),
    })
}
