use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// An incoming message from a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    /// Channel-specific message ID.
    pub id: String,
    /// Channel identifier (e.g., "telegram", "discord").
    pub channel: String,
    /// Sender identifier (channel-specific).
    pub sender: String,
    /// Display name of the sender.
    pub sender_name: Option<String>,
    /// Group/chat identifier (None for DMs).
    pub group: Option<String>,
    /// Text content.
    pub text: Option<String>,
    /// Attachments (images, files, audio, etc.)
    pub attachments: Vec<Attachment>,
    /// Whether the bot was explicitly mentioned.
    pub is_mention: bool,
    /// Whether this is a reply to the bot's message.
    pub is_reply_to_bot: bool,
    /// Raw channel-specific metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// An outgoing message to send via a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Target channel.
    pub channel: String,
    /// Target chat/user/group ID.
    pub target: String,
    /// Text content (may contain markdown).
    pub text: String,
    /// Attachments to send.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Reply to a specific message ID.
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub media_type: String,
    /// Base64 data or URL.
    pub data: String,
}

/// Events emitted by a channel adapter.
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A new message arrived.
    Message(IncomingMessage),
    /// The channel connected successfully.
    Connected,
    /// The channel disconnected.
    Disconnected(Option<String>),
    /// A typing indicator was received.
    Typing {
        sender: String,
        group: Option<String>,
    },
    /// A reaction was added.
    Reaction {
        message_id: String,
        sender: String,
        emoji: String,
    },
    /// A callback query (e.g. Telegram inline keyboard button press).
    CallbackQuery {
        /// Channel-specific callback ID (for answering the callback).
        callback_id: String,
        /// The data payload from the button.
        data: String,
        /// Who pressed the button.
        sender: String,
        /// Which chat the button was pressed in.
        chat_id: String,
    },
}

/// An approval prompt to send to a channel with approve/deny actions.
#[derive(Debug, Clone)]
pub struct ApprovalPrompt {
    /// Unique approval ID.
    pub approval_id: String,
    /// Target chat/user to send the prompt to.
    pub target: String,
    /// The tool that needs approval.
    pub tool_name: String,
    /// The tool arguments.
    pub tool_args: serde_json::Value,
    /// Why escalation was triggered.
    pub reason: String,
    /// Risk level 0-10.
    pub risk_level: u8,
}

/// Trait implemented by each channel adapter.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique identifier for this channel instance.
    fn id(&self) -> &str;

    /// Channel type name (e.g., "telegram", "discord").
    fn channel_type(&self) -> &str;

    /// Start the channel adapter. Returns a receiver for incoming events.
    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>>;

    /// Send a message through this channel.
    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()>;

    /// Send a typing indicator.
    async fn send_typing(&self, target: &str) -> claw_core::Result<()>;

    /// Send a message and return its platform-specific message ID (for later editing).
    /// Default implementation delegates to `send()` and returns `None`.
    async fn send_returning_id(
        &self,
        message: OutgoingMessage,
    ) -> claw_core::Result<Option<String>> {
        self.send(message).await?;
        Ok(None)
    }

    /// Edit a previously sent message by its platform-specific message ID.
    /// Default implementation is a no-op (channels that don't support editing).
    async fn edit_message(
        &self,
        _target: &str,
        _message_id: &str,
        _text: &str,
    ) -> claw_core::Result<()> {
        Ok(())
    }

    /// Send an approval prompt with approve/deny buttons.
    /// Default implementation sends a text-only message.
    async fn send_approval_prompt(&self, prompt: ApprovalPrompt) -> claw_core::Result<()> {
        let args_preview = serde_json::to_string_pretty(&prompt.tool_args)
            .unwrap_or_else(|_| prompt.tool_args.to_string());
        let text = format!(
            "⚠️ *Approval Required*\n\n\
             🔧 Tool: `{}`\n\
             ⚡ Risk: {}/10\n\
             📋 Reason: {}\n\
             ```\n{}\n```\n\n\
             _Reply with /approve {} or /deny {}_",
            prompt.tool_name,
            prompt.risk_level,
            prompt.reason,
            args_preview,
            prompt.approval_id,
            prompt.approval_id,
        );
        self.send(OutgoingMessage {
            channel: self.id().to_string(),
            target: prompt.target,
            text,
            attachments: vec![],
            reply_to: None,
        })
        .await
    }

    /// Stop the channel adapter gracefully.
    async fn stop(&mut self) -> claw_core::Result<()>;

    /// Check if the channel is currently connected.
    fn is_connected(&self) -> bool;
}
