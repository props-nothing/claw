use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::sync::Arc;
use tokio::sync::broadcast;

/// Events flowing through the system — the central nervous system of Claw.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    // ── Message lifecycle ──────────────────────────────────────
    MessageReceived {
        session_id: Uuid,
        message_id: Uuid,
        channel: String,
    },
    MessageSent {
        session_id: Uuid,
        message_id: Uuid,
        channel: String,
    },

    // ── Agent lifecycle ────────────────────────────────────────
    AgentThinking {
        session_id: Uuid,
    },
    AgentToolCall {
        session_id: Uuid,
        tool_name: String,
        tool_call_id: String,
    },
    AgentToolResult {
        session_id: Uuid,
        tool_call_id: String,
        is_error: bool,
    },
    AgentResponse {
        session_id: Uuid,
        message_id: Uuid,
    },
    AgentError {
        session_id: Uuid,
        error: String,
    },

    // ── Goal / autonomy lifecycle ──────────────────────────────
    GoalCreated {
        goal_id: Uuid,
        description: String,
    },
    GoalProgress {
        goal_id: Uuid,
        progress: f32,
        status: String,
    },
    GoalCompleted {
        goal_id: Uuid,
    },
    GoalFailed {
        goal_id: Uuid,
        reason: String,
    },
    ApprovalRequested {
        request_id: Uuid,
        action: String,
        reason: String,
    },
    ApprovalGranted {
        request_id: Uuid,
    },
    ApprovalDenied {
        request_id: Uuid,
    },

    // ── Plugin lifecycle ───────────────────────────────────────
    PluginLoaded {
        plugin_id: String,
        version: String,
    },
    PluginUnloaded {
        plugin_id: String,
    },
    PluginError {
        plugin_id: String,
        error: String,
    },

    // ── Mesh / peer lifecycle ──────────────────────────────────
    PeerJoined {
        peer_id: String,
        capabilities: Vec<String>,
    },
    PeerLeft {
        peer_id: String,
    },
    TaskDelegated {
        task_id: Uuid,
        peer_id: String,
        description: String,
    },

    // ── Channel lifecycle ──────────────────────────────────────
    ChannelConnected {
        channel_id: String,
        channel_type: String,
    },
    ChannelDisconnected {
        channel_id: String,
    },

    // ── System ─────────────────────────────────────────────────
    Heartbeat {
        timestamp: DateTime<Utc>,
    },
    Shutdown,
}

/// A broadcast-based event bus for system-wide pub/sub.
#[derive(Clone)]
pub struct EventBus {
    sender: Arc<broadcast::Sender<Event>>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
        }
    }

    pub fn publish(&self, event: Event) {
        // Ignore send errors (no subscribers).
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(4096)
    }
}
