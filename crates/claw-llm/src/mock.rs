//! Mock LLM provider for deterministic testing.
//!
//! Returns pre-configured responses without making any HTTP calls.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::provider::*;
use claw_core::{Message, MessageContent, Result, Role};

/// A mock LLM provider that returns pre-configured responses.
///
/// # Example
/// ```
/// use claw_llm::mock::MockProvider;
/// let provider = MockProvider::new("test")
///     .with_response("Hello, world!");
/// ```
pub struct MockProvider {
    responses: Arc<Mutex<Vec<MockResponse>>>,
    /// Track all requests received (for assertions in tests).
    pub requests: Arc<Mutex<Vec<LlmRequest>>>,
    name: String,
}

/// A pre-configured response from the mock provider.
#[derive(Clone)]
pub struct MockResponse {
    pub text: String,
    pub tool_calls: Vec<claw_core::ToolCall>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// If set, the provider will return this error instead.
    pub error: Option<String>,
}

impl Default for MockResponse {
    fn default() -> Self {
        Self {
            text: String::new(),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                thinking_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                estimated_cost_usd: 0.001,
            },
            error: None,
        }
    }
}

impl MockResponse {
    /// Create a text response.
    pub fn text(text: &str) -> Self {
        Self {
            text: text.to_string(),
            ..Default::default()
        }
    }

    /// Create an error response.
    pub fn error(msg: &str) -> Self {
        Self {
            error: Some(msg.to_string()),
            ..Default::default()
        }
    }
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(vec![])),
            requests: Arc::new(Mutex::new(vec![])),
            name: name.into(),
        }
    }

    /// Queue a simple text response.
    pub fn with_response(self, text: &str) -> Self {
        self.responses.lock().unwrap().push(MockResponse {
            text: text.to_string(),
            ..Default::default()
        });
        self
    }

    /// Queue a tool call response.
    pub fn with_tool_call(self, name: &str, args: serde_json::Value) -> Self {
        self.responses.lock().unwrap().push(MockResponse {
            tool_calls: vec![claw_core::ToolCall {
                id: format!("call_{}", uuid::Uuid::new_v4()),
                tool_name: name.to_string(),
                arguments: args,
            }],
            stop_reason: StopReason::ToolUse,
            ..Default::default()
        });
        self
    }

    /// Queue an error response.
    pub fn with_error(self, error: &str) -> Self {
        self.responses.lock().unwrap().push(MockResponse {
            error: Some(error.to_string()),
            ..Default::default()
        });
        self
    }

    /// Queue a fully custom response.
    pub fn with_mock_response(self, resp: MockResponse) -> Self {
        self.responses.lock().unwrap().push(resp);
        self
    }

    /// Get all requests that were made to this provider.
    pub fn recorded_requests(&self) -> Arc<Mutex<Vec<LlmRequest>>> {
        Arc::clone(&self.requests)
    }

    /// Queue a response directly (for mutable access patterns).
    pub fn queue_response(&mut self, resp: MockResponse) {
        self.responses.lock().unwrap().push(resp);
    }

    /// Pop the next queued response, or return a default "no response queued" message.
    fn next_response(&self) -> MockResponse {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            MockResponse {
                text: "(mock: no more queued responses)".to_string(),
                ..Default::default()
            }
        } else {
            responses.remove(0)
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn models(&self) -> Vec<String> {
        vec!["mock/test-model".to_string()]
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        self.requests.lock().unwrap().push(request.clone());
        let mock = self.next_response();

        if let Some(error) = mock.error {
            return Err(claw_core::ClawError::LlmProvider(error));
        }

        let mut content = vec![];
        if !mock.text.is_empty() {
            content.push(MessageContent::Text { text: mock.text });
        }

        let has_tool_calls = !mock.tool_calls.is_empty();

        let mut msg = Message::text(uuid::Uuid::nil(), Role::Assistant, "");
        msg.content = content;
        msg.tool_calls = mock.tool_calls;

        Ok(LlmResponse {
            message: msg,
            usage: mock.usage,
            has_tool_calls,
            stop_reason: mock.stop_reason,
        })
    }

    async fn stream(&self, request: &LlmRequest) -> Result<mpsc::Receiver<StreamChunk>> {
        self.requests.lock().unwrap().push(request.clone());
        let mock = self.next_response();

        let (tx, rx) = mpsc::channel(64);

        if let Some(error) = mock.error {
            tokio::spawn(async move {
                let _ = tx.send(StreamChunk::Error(error)).await;
            });
            return Ok(rx);
        }

        tokio::spawn(async move {
            // Stream the text word by word
            if !mock.text.is_empty() {
                for word in mock.text.split_whitespace() {
                    let _ = tx.send(StreamChunk::TextDelta(format!("{} ", word))).await;
                }
            }

            // Send tool calls
            for tc in mock.tool_calls {
                let _ = tx.send(StreamChunk::ToolCall(tc)).await;
            }

            // Send usage
            let _ = tx.send(StreamChunk::Usage(mock.usage)).await;

            // Done
            let _ = tx.send(StreamChunk::Done(mock.stop_reason)).await;
        });

        Ok(rx)
    }

    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_text_response() {
        let provider = MockProvider::new("mock").with_response("Hello!");
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            system: None,
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: false,
        };

        let resp = provider.complete(&req).await.unwrap();
        assert_eq!(resp.message.text_content(), "Hello!");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert!(!resp.has_tool_calls);
    }

    #[tokio::test]
    async fn test_mock_tool_call() {
        let provider = MockProvider::new("mock")
            .with_tool_call("shell_exec", serde_json::json!({"command": "ls"}));
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            system: None,
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: false,
        };

        let resp = provider.complete(&req).await.unwrap();
        assert!(resp.has_tool_calls);
        assert_eq!(resp.message.tool_calls[0].tool_name, "shell_exec");
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
    }

    #[tokio::test]
    async fn test_mock_error() {
        let provider = MockProvider::new("mock").with_error("HTTP 429: rate limited");
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            system: None,
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: false,
        };

        let result = provider.complete(&req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_records_requests() {
        let provider = MockProvider::new("mock").with_response("ok");
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![Message::text(uuid::Uuid::nil(), Role::User, "hello")],
            tools: vec![],
            system: Some("be nice".into()),
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: false,
        };

        let _ = provider.complete(&req).await;
        let recorded = provider.recorded_requests();
        let recorded = recorded.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].system, Some("be nice".into()));
    }

    #[tokio::test]
    async fn test_mock_streaming() {
        let provider = MockProvider::new("mock").with_response("Hello world");
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            system: None,
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: true,
        };

        let mut rx = provider.stream(&req).await.unwrap();
        let mut chunks = vec![];
        while let Some(chunk) = rx.recv().await {
            chunks.push(chunk);
        }
        // Should have TextDelta chunks + Usage + Done
        assert!(chunks.len() >= 3);
        assert!(matches!(chunks.last().unwrap(), StreamChunk::Done(_)));
    }

    #[tokio::test]
    async fn test_mock_multiple_responses_in_order() {
        let provider = MockProvider::new("mock")
            .with_response("first")
            .with_response("second")
            .with_response("third");
        let req = LlmRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            system: None,
            max_tokens: 100,
            temperature: 0.7,
            thinking_level: None,
            stream: false,
        };

        let r1 = provider.complete(&req).await.unwrap();
        let r2 = provider.complete(&req).await.unwrap();
        let r3 = provider.complete(&req).await.unwrap();
        assert_eq!(r1.message.text_content(), "first");
        assert_eq!(r2.message.text_content(), "second");
        assert_eq!(r3.message.text_content(), "third");
    }
}
