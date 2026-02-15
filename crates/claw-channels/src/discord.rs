use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::*;

/// Discord Gateway opcodes.
const OP_DISPATCH: u64 = 0;
const OP_HEARTBEAT: u64 = 1;
const OP_IDENTIFY: u64 = 2;
const OP_HELLO: u64 = 10;
const OP_HEARTBEAT_ACK: u64 = 11;

const DISCORD_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord channel adapter using Gateway WebSocket for receiving and REST API for sending.
///
/// ## Setup
///
/// 1. Go to <https://discord.com/developers/applications> → Create application
/// 2. Bot section → Copy token
/// 3. Enable "Message Content Intent" under Privileged Gateway Intents
/// 4. OAuth2 URL Generator → bot scope → Send Messages permission → invite to server
/// 5. Configure in claw.toml:
///    ```toml
///    [channels.discord]
///    type = "discord"
///    token = "YOUR_BOT_TOKEN"
///    ```
pub struct DiscordChannel {
    id: String,
    token: String,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    bot_user_id: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl DiscordChannel {
    pub fn new(id: String, token: String) -> Self {
        Self {
            id,
            token,
            client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            bot_user_id: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn id(&self) -> &str {
        &self.id
    }
    fn channel_type(&self) -> &str {
        "discord"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let token = self.token.clone();
        let connected = self.connected.clone();
        let channel_id = self.id.clone();
        let bot_user_id = self.bot_user_id.clone();

        tokio::spawn(async move {
            discord_gateway_loop(
                token,
                channel_id,
                event_tx,
                shutdown_rx,
                connected,
                bot_user_id,
            )
            .await;
        });

        Ok(event_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, message.target);

        let body = json!({
            "content": message.text,
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "discord".into(),
                reason: format!("HTTP error: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "Discord API error sending message");
            return Err(claw_core::ClawError::Channel {
                channel: "discord".into(),
                reason: format!("Discord API {status} — {text}"),
            });
        }

        Ok(())
    }

    async fn send_typing(&self, target: &str) -> claw_core::Result<()> {
        let url = format!("{DISCORD_API_BASE}/channels/{target}/typing");

        let _ = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .send()
            .await;

        Ok(())
    }

    async fn send_returning_id(
        &self,
        message: OutgoingMessage,
    ) -> claw_core::Result<Option<String>> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, message.target);

        let body = json!({ "content": message.text });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "discord".into(),
                reason: format!("HTTP error: {e}"),
            })?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::Channel {
                channel: "discord".into(),
                reason: text,
            });
        }

        let data: Value = resp.json().await.unwrap_or_default();
        Ok(data["id"].as_str().map(|s| s.to_string()))
    }

    async fn edit_message(
        &self,
        target: &str,
        message_id: &str,
        text: &str,
    ) -> claw_core::Result<()> {
        let url = format!("{DISCORD_API_BASE}/channels/{target}/messages/{message_id}");

        let body = json!({ "content": text });

        let _ = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        self.connected.store(false, Ordering::SeqCst);
        info!("Discord channel stopped");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Main gateway loop: connects to Discord WebSocket, handles heartbeats, dispatches events.
async fn discord_gateway_loop(
    token: String,
    channel_id: String,
    event_tx: mpsc::Sender<ChannelEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    connected: Arc<AtomicBool>,
    bot_user_id: Arc<tokio::sync::RwLock<Option<String>>>,
) {
    let mut backoff = 1u64;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        info!("Discord: connecting to Gateway...");

        let ws_result = tokio_tungstenite::connect_async(DISCORD_GATEWAY_URL).await;

        let ws_stream = match ws_result {
            Ok((stream, _)) => stream,
            Err(e) => {
                error!(error = %e, "Discord Gateway connection failed");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
        };

        backoff = 1;
        let (mut write, mut read) = ws_stream.split();

        // Wait for HELLO to get heartbeat interval
        let heartbeat_interval = match read.next().await {
            Some(Ok(msg)) => {
                let text = msg.to_text().unwrap_or("{}");
                let payload: Value = serde_json::from_str(text).unwrap_or_default();
                if payload["op"].as_u64() == Some(OP_HELLO) {
                    payload["d"]["heartbeat_interval"].as_u64().unwrap_or(41250)
                } else {
                    warn!("Discord: expected HELLO, got op={}", payload["op"]);
                    41250
                }
            }
            _ => {
                error!("Discord: no HELLO received");
                continue;
            }
        };

        // Send IDENTIFY
        let identify = json!({
            "op": OP_IDENTIFY,
            "d": {
                "token": token,
                "intents": 33281, // GUILDS | GUILD_MESSAGES | MESSAGE_CONTENT | DIRECT_MESSAGES
                "properties": {
                    "os": std::env::consts::OS,
                    "browser": "claw",
                    "device": "claw"
                }
            }
        });

        if let Err(e) = write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                identify.to_string().into(),
            ))
            .await
        {
            error!(error = %e, "Discord: failed to send IDENTIFY");
            continue;
        }

        connected.store(true, Ordering::SeqCst);
        let _ = event_tx.send(ChannelEvent::Connected).await;
        info!(
            heartbeat_ms = heartbeat_interval,
            "Discord Gateway connected"
        );

        let mut sequence: Option<u64> = None;
        let mut heartbeat_timer =
            tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));
        heartbeat_timer.tick().await; // consume initial tick

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Discord: shutdown signal received");
                        let _ = write.close().await;
                        return;
                    }
                }
                _ = heartbeat_timer.tick() => {
                    let hb = json!({ "op": OP_HEARTBEAT, "d": sequence });
                    if let Err(e) = write.send(
                        tokio_tungstenite::tungstenite::Message::Text(hb.to_string().into())
                    ).await {
                        warn!(error = %e, "Discord: heartbeat send failed");
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(ws_msg)) => {
                            if ws_msg.is_close() {
                                info!("Discord: server closed connection");
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

                            let op = payload["op"].as_u64().unwrap_or(999);

                            if let Some(s) = payload["s"].as_u64() {
                                sequence = Some(s);
                            }

                            match op {
                                OP_DISPATCH => {
                                    let event_name = payload["t"].as_str().unwrap_or("");
                                    let data = &payload["d"];
                                    handle_discord_dispatch(
                                        event_name, data, &channel_id,
                                        &event_tx, &bot_user_id,
                                    ).await;
                                }
                                OP_HEARTBEAT_ACK => {
                                    debug!("Discord: heartbeat ACK");
                                }
                                OP_HEARTBEAT => {
                                    let hb = json!({ "op": OP_HEARTBEAT, "d": sequence });
                                    let _ = write.send(
                                        tokio_tungstenite::tungstenite::Message::Text(hb.to_string().into())
                                    ).await;
                                }
                                _ => {
                                    debug!(op = op, "Discord: unhandled opcode");
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!(error = %e, "Discord WebSocket error");
                            break;
                        }
                        None => {
                            info!("Discord: WebSocket stream ended");
                            break;
                        }
                    }
                }
            }
        }

        connected.store(false, Ordering::SeqCst);
        let _ = event_tx
            .send(ChannelEvent::Disconnected(Some(
                "Gateway connection lost".into(),
            )))
            .await;

        if *shutdown_rx.borrow() {
            break;
        }

        info!(retry_in = backoff, "Discord: reconnecting...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

/// Handle a Discord DISPATCH event (op=0).
async fn handle_discord_dispatch(
    event_name: &str,
    data: &Value,
    channel_id: &str,
    event_tx: &mpsc::Sender<ChannelEvent>,
    bot_user_id: &Arc<tokio::sync::RwLock<Option<String>>>,
) {
    match event_name {
        "READY" => {
            if let Some(user_id) = data["user"]["id"].as_str() {
                let mut lock = bot_user_id.write().await;
                *lock = Some(user_id.to_string());
                info!(bot_id = %user_id, "Discord bot ready");
            }
        }
        "MESSAGE_CREATE" => {
            let author_id = data["author"]["id"].as_str().unwrap_or("");
            // Ignore our own messages
            {
                let lock = bot_user_id.read().await;
                if let Some(ref my_id) = *lock
                    && author_id == my_id {
                        return;
                    }
            }
            // Also ignore bot messages
            if data["author"]["bot"].as_bool().unwrap_or(false) {
                return;
            }

            let msg_id = data["id"].as_str().unwrap_or("").to_string();
            let author_name = data["author"]["username"].as_str().unwrap_or("unknown");
            let content = data["content"].as_str().unwrap_or("");
            let channel_ref = data["channel_id"].as_str().unwrap_or("");
            let guild_id = data["guild_id"].as_str().map(|s| s.to_string());

            let is_mention = {
                let lock = bot_user_id.read().await;
                lock.as_ref()
                    .map(|my_id| {
                        data["mentions"]
                            .as_array()
                            .map(|arr| arr.iter().any(|m| m["id"].as_str() == Some(my_id.as_str())))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            };

            // Check if replying to the bot
            let is_reply_to_bot = {
                let lock = bot_user_id.read().await;
                lock.as_ref()
                    .map(|my_id| {
                        data["referenced_message"]["author"]["id"].as_str() == Some(my_id.as_str())
                    })
                    .unwrap_or(false)
            };

            let is_dm = guild_id.is_none();

            let incoming = IncomingMessage {
                id: msg_id,
                channel: channel_id.to_string(),
                sender: author_id.to_string(),
                sender_name: Some(author_name.to_string()),
                group: if is_dm {
                    None
                } else {
                    Some(channel_ref.to_string())
                },
                text: Some(content.to_string()),
                attachments: parse_discord_attachments(data),
                is_mention,
                is_reply_to_bot,
                metadata: data.clone(),
            };

            debug!(sender = %author_name, channel = %channel_ref, dm = is_dm, "Discord message received");

            if event_tx
                .send(ChannelEvent::Message(incoming))
                .await
                .is_err()
            {
                warn!("Discord: event channel closed");
            }
        }
        "TYPING_START" => {
            let user_id = data["user_id"].as_str().unwrap_or("").to_string();
            let guild = data["guild_id"].as_str().map(|s| s.to_string());
            let _ = event_tx
                .send(ChannelEvent::Typing {
                    sender: user_id,
                    group: guild,
                })
                .await;
        }
        "MESSAGE_REACTION_ADD" => {
            let msg_id = data["message_id"].as_str().unwrap_or("").to_string();
            let user_id = data["user_id"].as_str().unwrap_or("").to_string();
            let emoji = data["emoji"]["name"].as_str().unwrap_or("").to_string();
            let _ = event_tx
                .send(ChannelEvent::Reaction {
                    message_id: msg_id,
                    sender: user_id,
                    emoji,
                })
                .await;
        }
        _ => {
            debug!(event = %event_name, "Discord: unhandled dispatch event");
        }
    }
}

/// Extract file attachments from a Discord MESSAGE_CREATE payload.
fn parse_discord_attachments(data: &Value) -> Vec<Attachment> {
    let mut result = Vec::new();
    if let Some(attachments) = data["attachments"].as_array() {
        for att in attachments {
            let filename = att["filename"].as_str().unwrap_or("file").to_string();
            let content_type = att["content_type"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .to_string();
            let url = att["url"].as_str().unwrap_or("").to_string();
            if !url.is_empty() {
                result.push(Attachment {
                    filename,
                    media_type: content_type,
                    data: url,
                });
            }
        }
    }
    result
}
