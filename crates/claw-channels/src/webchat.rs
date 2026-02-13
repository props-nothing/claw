use async_trait::async_trait;
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::adapter::*;

/// WebChat channel — messages come in via the HTTP/WebSocket API server.
/// This is the simplest channel, used for the built-in web UI and API access.
pub struct WebChatChannel {
    id: String,
    connected: Arc<AtomicBool>,
    /// Sender for outgoing messages (consumed by the server).
    outgoing_tx: Option<mpsc::Sender<OutgoingMessage>>,
    /// Receiver for outgoing messages (used by the server to forward).
    outgoing_rx: Option<mpsc::Receiver<OutgoingMessage>>,
    /// Sender for incoming events (used by the server to inject messages).
    incoming_tx: Option<mpsc::Sender<ChannelEvent>>,
}

impl WebChatChannel {
    pub fn new(id: String) -> Self {
        let (outgoing_tx, outgoing_rx) = mpsc::channel(256);
        Self {
            id,
            connected: Arc::new(AtomicBool::new(false)),
            outgoing_tx: Some(outgoing_tx),
            outgoing_rx: Some(outgoing_rx),
            incoming_tx: None,
        }
    }

    /// Get a sender that the HTTP server can use to inject incoming messages.
    pub fn incoming_sender(&self) -> Option<mpsc::Sender<ChannelEvent>> {
        self.incoming_tx.clone()
    }

    /// Take the outgoing message receiver (used by the server).
    pub fn take_outgoing(&mut self) -> Option<mpsc::Receiver<OutgoingMessage>> {
        self.outgoing_rx.take()
    }
}

#[async_trait]
impl Channel for WebChatChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> &str {
        "webchat"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (incoming_tx, incoming_rx) = mpsc::channel(256);
        self.incoming_tx = Some(incoming_tx);
        self.connected.store(true, Ordering::SeqCst);
        Ok(incoming_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        if let Some(ref tx) = self.outgoing_tx {
            tx.send(message).await.map_err(|e| claw_core::ClawError::Channel {
                channel: "webchat".into(),
                reason: e.to_string(),
            })?;
        }
        Ok(())
    }

    async fn send_typing(&self, _target: &str) -> claw_core::Result<()> {
        // WebChat handles typing indicators via WebSocket events
        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.incoming_tx = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}
