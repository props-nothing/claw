use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::info;
use uuid::Uuid;

/// A request for human approval of an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub action: String,
    pub reason: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub risk_level: u8,
    pub created_at: DateTime<Utc>,
    /// Timeout in seconds — auto-deny after this.
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResponse {
    Approved,
    Denied,
    TimedOut,
}

/// The approval gate manages pending approval requests.
/// It sends requests to all connected channels and waits for a response.
pub struct ApprovalGate {
    /// Sender for new approval requests that need to be shown to the user.
    request_tx: mpsc::Sender<(ApprovalRequest, oneshot::Sender<ApprovalResponse>)>,
    /// Receiver side — consumed by the server/channel layer.
    request_rx: Option<mpsc::Receiver<(ApprovalRequest, oneshot::Sender<ApprovalResponse>)>>,
}

impl Default for ApprovalGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalGate {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            request_tx: tx,
            request_rx: Some(rx),
        }
    }

    /// Take the receiver (used by the server to listen for approval requests).
    pub fn take_receiver(
        &mut self,
    ) -> Option<mpsc::Receiver<(ApprovalRequest, oneshot::Sender<ApprovalResponse>)>> {
        self.request_rx.take()
    }

    /// Request approval for an action. Blocks until approved, denied, or timeout.
    pub async fn request_approval(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
        reason: &str,
        risk_level: u8,
        timeout_secs: u64,
    ) -> ApprovalResponse {
        let id = Uuid::new_v4();
        self.request_approval_with_id(id, tool_name, tool_args, reason, risk_level, timeout_secs)
            .await
    }

    /// Request approval with a specific pre-generated ID (so callers can emit the ID beforehand).
    pub async fn request_approval_with_id(
        &self,
        id: Uuid,
        tool_name: &str,
        tool_args: &serde_json::Value,
        reason: &str,
        risk_level: u8,
        timeout_secs: u64,
    ) -> ApprovalResponse {
        let request = ApprovalRequest {
            id,
            action: format!("{tool_name}({tool_args})"),
            reason: reason.to_string(),
            tool_name: tool_name.to_string(),
            tool_args: tool_args.clone(),
            risk_level,
            created_at: Utc::now(),
            timeout_secs,
        };

        info!(
            request_id = %request.id,
            tool = tool_name,
            risk = risk_level,
            "requesting human approval"
        );

        let (response_tx, response_rx) = oneshot::channel();

        // Send the request to the approval channel
        if self.request_tx.send((request, response_tx)).await.is_err() {
            // No one listening — auto-deny
            return ApprovalResponse::Denied;
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), response_rx).await
        {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => ApprovalResponse::Denied, // channel closed
            Err(_) => {
                info!("approval request timed out");
                ApprovalResponse::TimedOut
            }
        }
    }
}
