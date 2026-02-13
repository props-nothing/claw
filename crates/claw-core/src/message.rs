use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: Role,
    pub content: Vec<MessageContent>,
    pub timestamp: DateTime<Utc>,
    /// Tool calls requested by the assistant in this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<super::tool::ToolCall>,
    /// Optional metadata (channel source, peer id, etc.)
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },
    Image {
        /// Base64‐encoded image data or a URL.
        data: String,
        media_type: String,
    },
    Audio {
        data: String,
        media_type: String,
    },
    File {
        path: String,
        media_type: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
}

impl Message {
    /// Create a simple text message.
    pub fn text(session_id: Uuid, role: Role, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role,
            content: vec![MessageContent::Text { text: text.into() }],
            timestamp: Utc::now(),
            tool_calls: vec![],
            metadata: Default::default(),
        }
    }

    /// Extract all text content joined together.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                MessageContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Estimate token count for this message.
    /// Uses a simple heuristic: ~4 chars per token for English text.
    /// Includes tool call arguments and tool result content.
    pub fn estimate_tokens(&self) -> usize {
        let mut chars = 0usize;

        // Role overhead (~4 tokens for role markers)
        chars += 16;

        // Content blocks
        for block in &self.content {
            match block {
                MessageContent::Text { text } => chars += text.len(),
                MessageContent::ToolResult { content, tool_call_id, .. } => {
                    chars += content.len();
                    chars += tool_call_id.len();
                }
                MessageContent::Image { data, .. } => chars += data.len().min(1000),
                MessageContent::Audio { data, .. } => chars += data.len().min(1000),
                MessageContent::File { path, .. } => chars += path.len(),
            }
        }

        // Tool calls (function name + JSON arguments)
        for tc in &self.tool_calls {
            chars += tc.tool_name.len();
            chars += tc.id.len();
            chars += tc.arguments.to_string().len();
        }

        // ~4 chars per token, minimum 1
        (chars / 4).max(1)
    }
}
