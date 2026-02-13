use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Description of a tool that can be called by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Unique name, e.g. "browser.navigate", "shell.exec", "file.read".
    pub name: String,
    /// Human-readable description for the LLM.
    pub description: String,
    /// JSON Schema of the parameters object.
    pub parameters: Value,
    /// Required capabilities to execute (used by the guardrail system).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Whether this tool has side-effects (write vs read).
    #[serde(default)]
    pub is_mutating: bool,
    /// Risk level 0-10 (used by autonomy system to decide approval).
    #[serde(default)]
    pub risk_level: u8,
    /// Which plugin provides this tool (None = built-in).
    #[serde(default)]
    pub provider: Option<String>,
}

/// A request from the LLM to call a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// The result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
    /// Optional structured data returned alongside the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Trait implemented by anything that can execute tool calls.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// List all tools this executor provides.
    fn tools(&self) -> Vec<Tool>;

    /// Execute a single tool call and return the result.
    async fn execute(&self, call: &ToolCall) -> crate::Result<ToolResult>;
}
