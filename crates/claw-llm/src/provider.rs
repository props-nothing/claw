use async_trait::async_trait;
use claw_core::{Message, Result, Tool};
use serde::{Deserialize, Serialize};

/// A request to an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    /// The model to use, e.g. "claude-opus-4-6" (provider-specific part).
    pub model: String,
    /// Conversation history.
    pub messages: Vec<Message>,
    /// Available tools.
    pub tools: Vec<Tool>,
    /// System prompt (separate from messages for providers that support it).
    pub system: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Temperature.
    pub temperature: f32,
    /// Thinking level for extended reasoning ("off", "low", "medium", "high", "xhigh").
    pub thinking_level: Option<String>,
    /// Whether to stream the response.
    pub stream: bool,
}

/// A complete (non-streaming) response from an LLM.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub message: Message,
    pub usage: Usage,
    /// Whether the model wants to continue (has tool calls).
    pub has_tool_calls: bool,
    /// Stop reason.
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    ContentFilter,
}

/// A chunk of a streaming response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Thinking / reasoning text (shown to user as "thinking...").
    Thinking(String),
    /// Content text delta.
    TextDelta(String),
    /// A tool call was decided.
    ToolCall(claw_core::ToolCall),
    /// Usage stats (sent at end of stream).
    Usage(Usage),
    /// Stream is done.
    Done(StopReason),
    /// An error occurred mid-stream.
    Error(String),
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub thinking_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    /// Estimated cost in USD (computed by the provider adapter).
    pub estimated_cost_usd: f64,
}

impl Usage {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens + self.thinking_tokens
    }

    pub fn merge(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.thinking_tokens += other.thinking_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.estimated_cost_usd += other.estimated_cost_usd;
    }
}

/// Trait implemented by each LLM provider (Anthropic, OpenAI, local, etc.)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Human-readable name, e.g. "Anthropic", "OpenAI", "Local/llama.cpp"
    fn name(&self) -> &str;

    /// List available models.
    fn models(&self) -> Vec<String>;

    /// Send a non-streaming request.
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse>;

    /// Send a streaming request. Returns a receiver for chunks.
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>>;

    /// Check if this provider is healthy / reachable.
    async fn health_check(&self) -> Result<()>;
}
