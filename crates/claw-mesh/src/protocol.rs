use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages exchanged between mesh peers via GossipSub.
///
/// All mesh communication goes through the `claw/mesh/v1` GossipSub topic.
/// Messages addressed to a specific peer include a `to_peer` field; the
/// target processes them while others ignore them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeshMessage {
    /// Announce device capabilities (broadcast periodically + on join).
    Announce {
        peer_id: String,
        hostname: String,
        capabilities: Vec<String>,
        os: String,
    },
    /// Delegate a task to a specific peer.
    TaskAssign(TaskAssignment),
    /// Report task result back to the originator.
    TaskResult {
        task_id: Uuid,
        peer_id: String,
        success: bool,
        result: String,
    },
    /// Synchronize memory/state (CRDT delta).
    SyncDelta {
        peer_id: String,
        delta_type: String,
        data: serde_json::Value,
    },
    /// Heartbeat / keepalive.
    Ping { peer_id: String, timestamp: i64 },
    /// Response to ping.
    Pong { peer_id: String, timestamp: i64 },
    /// Free-form text message to a specific peer (used by `claw mesh send`).
    DirectMessage {
        from_peer: String,
        to_peer: String,
        content: String,
        timestamp: i64,
    },
    /// Peer disconnected (local-only, not sent over the wire).
    PeerLeft { peer_id: String },
}

/// A task delegated to a specific device in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignment {
    pub task_id: Uuid,
    /// Who is assigning the task.
    pub from_peer: String,
    /// Who should execute it.
    pub to_peer: String,
    /// What to do (human-readable description).
    pub description: String,
    /// Required capability (e.g., "camera", "gpu", "browser").
    pub required_capability: Option<String>,
    /// Tool call to execute.
    pub tool_call: Option<claw_core::ToolCall>,
    /// Priority (higher = more urgent).
    pub priority: u8,
}

impl TaskAssignment {
    /// Create a new task assignment.
    pub fn new(
        from_peer: impl Into<String>,
        to_peer: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            task_id: Uuid::new_v4(),
            from_peer: from_peer.into(),
            to_peer: to_peer.into(),
            description: description.into(),
            required_capability: None,
            tool_call: None,
            priority: 5,
        }
    }

    /// Set the required capability for this task.
    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.required_capability = Some(cap.into());
        self
    }

    /// Set a tool call to execute for this task.
    pub fn with_tool_call(mut self, tool_call: claw_core::ToolCall) -> Self {
        self.tool_call = Some(tool_call);
        self
    }

    /// Set the priority (0 = lowest, 10 = highest).
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority.min(10);
        self
    }
}

impl MeshMessage {
    /// Check if this message is addressed to a specific peer.
    /// Returns true if the message is a broadcast or addressed to the given peer.
    pub fn is_for_peer(&self, our_peer_id: &str) -> bool {
        match self {
            // Broadcasts — everyone processes these
            MeshMessage::Announce { .. }
            | MeshMessage::Ping { .. }
            | MeshMessage::Pong { .. }
            | MeshMessage::SyncDelta { .. }
            | MeshMessage::PeerLeft { .. } => true,

            // Directed messages — only the target processes these
            MeshMessage::TaskAssign(task) => task.to_peer == our_peer_id,
            MeshMessage::TaskResult { .. } => true, // originator processes the result
            MeshMessage::DirectMessage { to_peer, .. } => to_peer == our_peer_id,
        }
    }
}
