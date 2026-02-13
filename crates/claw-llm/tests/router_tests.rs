#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use claw_llm::mock::{MockProvider, MockResponse};
    use claw_llm::router::ModelRouter;
    use claw_llm::provider::LlmRequest;
    use claw_core::{Message, Role};
    use uuid::Uuid;

    fn make_request(model: &str) -> LlmRequest {
        LlmRequest {
            model: model.to_string(),
            messages: vec![Message::text(Uuid::nil(), Role::User, "Hello")],
            max_tokens: 100,
            temperature: 0.7,
            tools: vec![],
            system: None,
            stream: false,
            thinking_level: None,
        }
    }

    // ── Router resolve / complete ──────────────────────────────

    #[tokio::test]
    async fn test_complete_with_prefix_resolution() {
        let mock = MockProvider::new("testprovider");
        let mock = mock.with_response("Hello from mock!");
        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));
        let req = make_request("testprovider/gpt-4o");
        let resp = router.complete(&req, None).await.unwrap();
        assert_eq!(resp.message.text_content(), "Hello from mock!");
    }

    #[tokio::test]
    async fn test_model_not_found() {
        let router = ModelRouter::new();
        let req = make_request("nonexistent/model");
        let result = router.complete(&req, None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), claw_core::ClawError::ModelNotFound(_)));
    }

    #[tokio::test]
    async fn test_failover_to_fallback() {
        let mut primary = MockProvider::new("primary");
        primary.queue_response(MockResponse::error("HTTP 500: Internal Server Error"));
        primary.queue_response(MockResponse::error("HTTP 500: Internal Server Error"));
        primary.queue_response(MockResponse::error("HTTP 500: Internal Server Error"));
        primary.queue_response(MockResponse::error("HTTP 500: Internal Server Error"));

        let fallback = MockProvider::new("fallback").with_response("Fallback reply");

        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(primary));
        router.add_provider(Arc::new(fallback));

        let req = make_request("primary/model");
        let resp = router.complete(&req, Some("fallback/model")).await.unwrap();
        assert_eq!(resp.message.text_content(), "Fallback reply");
    }

    // ── Retry logic ────────────────────────────────────────────

    #[tokio::test]
    async fn test_retry_on_transient_error() {
        let mut mock = MockProvider::new("retry_test");
        // First call fails with retryable error, second succeeds
        mock.queue_response(MockResponse::error("HTTP 429: rate limited"));
        mock.queue_response(MockResponse::text("success after retry"));

        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));

        let req = make_request("retry_test/model");
        let resp = router.complete(&req, None).await.unwrap();
        assert_eq!(resp.message.text_content(), "success after retry");
    }

    #[tokio::test]
    async fn test_no_retry_on_non_transient_error() {
        let mut mock = MockProvider::new("no_retry");
        mock.queue_response(MockResponse::error("Invalid API key"));

        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));

        let req = make_request("no_retry/model");
        let result = router.complete(&req, None).await;
        assert!(result.is_err());
    }

    // ── Stream ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stream_basic() {
        let mock = MockProvider::new("stream_test").with_response("streamed text");
        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));

        let req = make_request("stream_test/model");
        let mut rx = router.stream(&req, None).await.unwrap();

        let mut text = String::new();
        while let Some(chunk) = rx.recv().await {
            if let claw_llm::provider::StreamChunk::TextDelta(t) = chunk {
                text.push_str(&t);
            }
        }
        assert!(!text.is_empty());
    }

    // ── Request recording ──────────────────────────────────────

    #[tokio::test]
    async fn test_request_recording() {
        let mock = MockProvider::new("recorder")
            .with_response("ok");
        let requests = mock.recorded_requests();

        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));

        let req = make_request("recorder/model");
        router.complete(&req, None).await.unwrap();

        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].messages[0].text_content(), "Hello");
    }
}
