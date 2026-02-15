use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::*;

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack channel adapter with Socket Mode for receiving and Web API for sending.
///
/// ## Setup
///
/// 1. Create a Slack App at <https://api.slack.com/apps>
/// 2. Enable **Socket Mode** and generate an App-Level Token (`xapp-...`) with `connections:write`
/// 3. Add Bot Token Scopes: `chat:write`, `channels:history`, `groups:history`, `im:history`
/// 4. Subscribe to events: `message.channels`, `message.groups`, `message.im`, `app_mention`
/// 5. Install to workspace and copy the Bot Token (`xoxb-...`)
/// 6. Configure in claw.toml:
///    ```toml
///    [channels.slack]
///    type = "slack"
///    token = "xoxb-..."
///    app_token = "xapp-..."
///    ```
pub struct SlackChannel {
    id: String,
    bot_token: String,
    app_token: Option<String>,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    bot_user_id: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl SlackChannel {
    pub fn new(id: String, bot_token: String, app_token: Option<String>) -> Self {
        Self {
            id,
            bot_token,
            app_token,
            client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            bot_user_id: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn id(&self) -> &str {
        &self.id
    }
    fn channel_type(&self) -> &str {
        "slack"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        // Fetch bot user ID via auth.test
        let bot_id = fetch_bot_user_id(&self.client, &self.bot_token).await;
        if let Some(ref id) = bot_id {
            let mut lock = self.bot_user_id.write().await;
            *lock = Some(id.clone());
            info!(bot_id = %id, "Slack bot authenticated");
        }

        let connected = self.connected.clone();

        if let Some(ref app_token) = self.app_token {
            // Socket Mode: real-time message receiving
            let app_token = app_token.clone();
            let bot_token = self.bot_token.clone();
            let channel_id = self.id.clone();
            let bot_user_id = self.bot_user_id.clone();
            let client = self.client.clone();

            tokio::spawn(async move {
                slack_socket_mode_loop(
                    app_token,
                    bot_token,
                    channel_id,
                    event_tx,
                    shutdown_rx,
                    connected,
                    bot_user_id,
                    client,
                )
                .await;
            });
        } else {
            // No app_token — can only send, not receive
            warn!(
                "Slack: no app_token configured — Socket Mode disabled. \
                   The agent can send messages but won't receive them. \
                   Add an app_token (xapp-...) to enable receiving."
            );
            connected.store(true, Ordering::SeqCst);
            let _ = event_tx.send(ChannelEvent::Connected).await;
        }

        Ok(event_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        let body = json!({
            "channel": message.target,
            "text": message.text,
        });

        let resp = self
            .client
            .post(format!("{SLACK_API_BASE}/chat.postMessage"))
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "slack".into(),
                reason: format!("HTTP error: {e}"),
            })?;

        let data: Value = resp.json().await.unwrap_or_default();
        if !data["ok"].as_bool().unwrap_or(false) {
            let err = data["error"].as_str().unwrap_or("unknown");
            warn!(error = %err, "Slack API error sending message");
            return Err(claw_core::ClawError::Channel {
                channel: "slack".into(),
                reason: format!("Slack API error: {err}"),
            });
        }

        Ok(())
    }

    async fn send_returning_id(
        &self,
        message: OutgoingMessage,
    ) -> claw_core::Result<Option<String>> {
        let body = json!({
            "channel": message.target,
            "text": message.text,
        });

        let resp = self
            .client
            .post(format!("{SLACK_API_BASE}/chat.postMessage"))
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "slack".into(),
                reason: format!("HTTP error: {e}"),
            })?;

        let data: Value = resp.json().await.unwrap_or_default();
        if !data["ok"].as_bool().unwrap_or(false) {
            let err = data["error"].as_str().unwrap_or("unknown");
            return Err(claw_core::ClawError::Channel {
                channel: "slack".into(),
                reason: format!("Slack API error: {err}"),
            });
        }

        Ok(data["ts"].as_str().map(|s| s.to_string()))
    }

    async fn edit_message(
        &self,
        target: &str,
        message_id: &str,
        text: &str,
    ) -> claw_core::Result<()> {
        let body = json!({
            "channel": target,
            "ts": message_id,
            "text": text,
        });

        let _ = self
            .client
            .post(format!("{SLACK_API_BASE}/chat.update"))
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn send_typing(&self, _target: &str) -> claw_core::Result<()> {
        // Slack doesn't have a typing indicator API for bots
        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        self.connected.store(false, Ordering::SeqCst);
        info!("Slack channel stopped");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Fetch the bot's own user ID via auth.test.
async fn fetch_bot_user_id(client: &reqwest::Client, bot_token: &str) -> Option<String> {
    let resp = client
        .post(format!("{SLACK_API_BASE}/auth.test"))
        .header("Authorization", format!("Bearer {bot_token}"))
        .send()
        .await
        .ok()?;

    let data: Value = resp.json().await.ok()?;
    data["user_id"].as_str().map(|s| s.to_string())
}

/// Socket Mode loop: opens a WebSocket to Slack for real-time event delivery.
#[allow(clippy::too_many_arguments)]
async fn slack_socket_mode_loop(
    app_token: String,
    _bot_token: String,
    channel_id: String,
    event_tx: mpsc::Sender<ChannelEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    connected: Arc<AtomicBool>,
    bot_user_id: Arc<tokio::sync::RwLock<Option<String>>>,
    client: reqwest::Client,
) {
    let mut backoff = 1u64;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        // Request a WebSocket URL via apps.connections.open
        let ws_url = match request_socket_mode_url(&client, &app_token).await {
            Some(url) => url,
            None => {
                error!("Slack: failed to get Socket Mode URL — check your app_token");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
        };

        info!("Slack: connecting to Socket Mode...");

        let ws_result = tokio_tungstenite::connect_async(&ws_url).await;

        let ws_stream = match ws_result {
            Ok((stream, _)) => stream,
            Err(e) => {
                error!(error = %e, "Slack Socket Mode connection failed");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
        };

        backoff = 1;
        connected.store(true, Ordering::SeqCst);
        let _ = event_tx.send(ChannelEvent::Connected).await;
        info!("Slack Socket Mode connected");

        let (mut write, mut read) = ws_stream.split();

        // Ping timer to keep connection alive
        let mut ping_timer = tokio::time::interval(std::time::Duration::from_secs(30));
        ping_timer.tick().await;

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        let _ = write.close().await;
                        return;
                    }
                }
                _ = ping_timer.tick() => {
                    let _ = write.send(
                        tokio_tungstenite::tungstenite::Message::Ping(vec![])
                    ).await;
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(ws_msg)) => {
                            if ws_msg.is_close() {
                                info!("Slack: Socket Mode connection closed by server");
                                break;
                            }
                            let text = match ws_msg.to_text() {
                                Ok(t) => t,
                                Err(_) => continue,
                            };
                            let payload: Value = match serde_json::from_str(text) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            let msg_type = payload["type"].as_str().unwrap_or("");

                            // Acknowledge the envelope immediately (Slack requires this)
                            if let Some(envelope_id) = payload["envelope_id"].as_str() {
                                let ack = json!({ "envelope_id": envelope_id });
                                let _ = write.send(
                                    tokio_tungstenite::tungstenite::Message::Text(ack.to_string())
                                ).await;
                            }

                            match msg_type {
                                "events_api" => {
                                    let event = &payload["payload"]["event"];
                                    let event_type = event["type"].as_str().unwrap_or("");

                                    match event_type {
                                        "message" => {
                                            // Skip bot messages, subtypes (edits, joins, etc.)
                                            if event.get("subtype").is_some() {
                                                continue;
                                            }
                                            let user = event["user"].as_str().unwrap_or("");
                                            // Skip our own messages
                                            {
                                                let lock = bot_user_id.read().await;
                                                if let Some(ref my_id) = *lock {
                                                    if user == my_id {
                                                        continue;
                                                    }
                                                }
                                            }

                                            let text = event["text"].as_str().unwrap_or("");
                                            let channel_ref = event["channel"].as_str().unwrap_or("");
                                            let ts = event["ts"].as_str().unwrap_or("");
                                            let channel_type = event["channel_type"].as_str().unwrap_or("");

                                            let is_dm = channel_type == "im";

                                            // Check for @mention of the bot
                                            let is_mention = {
                                                let lock = bot_user_id.read().await;
                                                lock.as_ref()
                                                    .map(|my_id| text.contains(&format!("<@{my_id}>")))
                                                    .unwrap_or(false)
                                            };

                                            let incoming = IncomingMessage {
                                                id: ts.to_string(),
                                                channel: channel_id.clone(),
                                                sender: user.to_string(),
                                                sender_name: None, // Slack doesn't include display name in events
                                                group: if is_dm { None } else { Some(channel_ref.to_string()) },
                                                text: Some(text.to_string()),
                                                attachments: parse_slack_files(event),
                                                is_mention,
                                                is_reply_to_bot: false,
                                                metadata: event.clone(),
                                            };

                                            debug!(sender = %user, channel = %channel_ref, "Slack message received");

                                            if event_tx.send(ChannelEvent::Message(incoming)).await.is_err() {
                                                warn!("Slack: event channel closed");
                                                return;
                                            }
                                        }
                                        "app_mention" => {
                                            let user = event["user"].as_str().unwrap_or("");
                                            let text = event["text"].as_str().unwrap_or("");
                                            let channel_ref = event["channel"].as_str().unwrap_or("");
                                            let ts = event["ts"].as_str().unwrap_or("");

                                            let incoming = IncomingMessage {
                                                id: ts.to_string(),
                                                channel: channel_id.clone(),
                                                sender: user.to_string(),
                                                sender_name: None,
                                                group: Some(channel_ref.to_string()),
                                                text: Some(text.to_string()),
                                                attachments: vec![],
                                                is_mention: true,
                                                is_reply_to_bot: false,
                                                metadata: event.clone(),
                                            };

                                            debug!(sender = %user, "Slack app_mention received");

                                            if event_tx.send(ChannelEvent::Message(incoming)).await.is_err() {
                                                return;
                                            }
                                        }
                                        "reaction_added" => {
                                            let _ = event_tx.send(ChannelEvent::Reaction {
                                                message_id: event["item"]["ts"].as_str().unwrap_or("").to_string(),
                                                sender: event["user"].as_str().unwrap_or("").to_string(),
                                                emoji: event["reaction"].as_str().unwrap_or("").to_string(),
                                            }).await;
                                        }
                                        _ => {
                                            debug!(event_type = %event_type, "Slack: unhandled event");
                                        }
                                    }
                                }
                                "disconnect" => {
                                    let reason = payload["reason"].as_str().unwrap_or("unknown");
                                    info!(reason = %reason, "Slack: server requested disconnect");
                                    break;
                                }
                                "hello" => {
                                    debug!("Slack: Socket Mode hello");
                                }
                                _ => {
                                    debug!(msg_type = %msg_type, "Slack: unhandled envelope type");
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!(error = %e, "Slack WebSocket error");
                            break;
                        }
                        None => {
                            info!("Slack: WebSocket stream ended");
                            break;
                        }
                    }
                }
            }
        }

        connected.store(false, Ordering::SeqCst);
        let _ = event_tx
            .send(ChannelEvent::Disconnected(Some(
                "Socket Mode connection lost".into(),
            )))
            .await;

        if *shutdown_rx.borrow() {
            break;
        }

        info!(retry_in = backoff, "Slack: reconnecting...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

/// Request a Socket Mode WebSocket URL via apps.connections.open.
async fn request_socket_mode_url(client: &reqwest::Client, app_token: &str) -> Option<String> {
    let resp = client
        .post(format!("{SLACK_API_BASE}/apps.connections.open"))
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .ok()?;

    let data: Value = resp.json().await.ok()?;
    if !data["ok"].as_bool().unwrap_or(false) {
        let err = data["error"].as_str().unwrap_or("unknown");
        error!(error = %err, "Slack apps.connections.open failed");
        return None;
    }

    data["url"].as_str().map(|s| s.to_string())
}

/// Extract file attachments from a Slack message event.
fn parse_slack_files(event: &Value) -> Vec<Attachment> {
    let mut result = Vec::new();
    if let Some(files) = event["files"].as_array() {
        for file in files {
            let name = file["name"].as_str().unwrap_or("file").to_string();
            let mimetype = file["mimetype"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .to_string();
            // url_private requires auth, but we store it for later download
            let url = file["url_private"]
                .as_str()
                .or_else(|| file["url_private_download"].as_str())
                .unwrap_or("")
                .to_string();
            if !url.is_empty() {
                result.push(Attachment {
                    filename: name,
                    media_type: mimetype,
                    data: url,
                });
            }
        }
    }
    result
}
