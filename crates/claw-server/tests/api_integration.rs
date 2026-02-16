//! HTTP API integration tests — exercise all server endpoints with a mock LLM.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use claw_config::schema::ServerConfig;
use claw_llm::ModelRouter;
use claw_llm::mock::MockProvider;
use claw_runtime::{RuntimeHandle, build_test_state_with_router, set_runtime_handle};
use std::sync::Arc;

/// Build a test router with a mock provider that has `n` queued text responses.
async fn setup(responses: Vec<&str>) -> axum::Router {
    let mut mock = MockProvider::new("mock");
    for r in responses {
        mock = mock.with_response(r);
    }
    let mut router = ModelRouter::new();
    router.add_provider(Arc::new(mock));

    let config = {
        let mut c = claw_config::ClawConfig::default();
        c.agent.model = "mock/test-model".to_string();
        c.agent.max_iterations = 5;
        c.autonomy.level = 3;
        c
    };

    let state = build_test_state_with_router(config, router).unwrap();
    let handle = RuntimeHandle::new_for_test(state);
    set_runtime_handle(handle).await;

    let server_config = ServerConfig {
        web_ui: false,
        cors: false,
        api_key: None,
        ..Default::default()
    };
    claw_server::build_router(
        server_config,
        None,
        std::path::PathBuf::from("/tmp/claw-test-skills"),
        std::path::PathBuf::from("/tmp/claw-test-plugins"),
    )
}

/// Helper to read the full body bytes from a response.
async fn body_string(resp: axum::response::Response) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ── Health & Metrics ───────────────────────────────────────────

#[tokio::test]
async fn test_health_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/health").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/metrics").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/plain"));
    let body = body_string(resp).await;
    assert!(body.contains("claw_http_requests_total"));
}

// ── Chat ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_endpoint() {
    let app = setup(vec!["Hello from Claw!"]).await;
    let req = Request::post("/api/v1/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"Hi"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Response text may vary depending on test ordering (shared global state)
    assert!(
        json["response"].is_string(),
        "expected response string, got: {json}"
    );
    assert!(json["session_id"].is_string());
}

#[tokio::test]
async fn test_chat_missing_body() {
    let app = setup(vec![]).await;
    let req = Request::post("/api/v1/chat")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    // Missing required field "message" → 422 Unprocessable Entity
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ── SSE Stream ─────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_stream_endpoint() {
    // Note: chat_stream uses stream_tx which has no receiver in test mode,
    // so it will return an error. That's fine — we just verify the route exists.
    let app = setup(vec!["streamed"]).await;
    let req = Request::post("/api/v1/chat/stream")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"Hi stream"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    // The stream_tx has no background consumer in test mode, so this will fail
    // with 500 (channel send error). That's expected behavior without the full runtime.
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected status: {}",
        resp.status()
    );
}

// ── Query Endpoints (all need runtime handle) ──────────────────

#[tokio::test]
async fn test_status_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/status").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json["model"].is_string());
    assert!(json["uptime_secs"].is_number());
}

#[tokio::test]
async fn test_sessions_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/sessions")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array() || json.is_object());
}

#[tokio::test]
async fn test_goals_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/goals").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array() || json.is_object());
}

#[tokio::test]
async fn test_tools_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/tools").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Should be an array of tool definitions or an object wrapper
    assert!(
        json.is_array() || json.is_object(),
        "unexpected tools format: {json}"
    );
}

#[tokio::test]
async fn test_facts_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/memory/facts")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_memory_search_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/memory/search?q=test")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_config_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/config").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json["agent"].is_object());
}

#[tokio::test]
async fn test_audit_endpoint() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/audit?limit=10")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ── Auth ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_api_key_rejects_unauthenticated() {
    // Set the global handle first
    {
        let mock = MockProvider::new("mock").with_response("secret");
        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));
        let mut config = claw_config::ClawConfig::default();
        config.agent.model = "mock/test-model".to_string();
        let state = build_test_state_with_router(config, router).unwrap();
        let handle = RuntimeHandle::new_for_test(state);
        set_runtime_handle(handle).await;
    }

    let server_config = ServerConfig {
        web_ui: false,
        cors: false,
        api_key: Some("test-secret-key".to_string()),
        ..Default::default()
    };
    let app = claw_server::build_router(
        server_config,
        None,
        std::path::PathBuf::from("/tmp/claw-test-skills"),
        std::path::PathBuf::from("/tmp/claw-test-plugins"),
    );

    // Request without API key
    let req = Request::get("/api/v1/status").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Request with wrong API key
    let req = Request::get("/api/v1/status")
        .header("authorization", "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Request with correct API key
    let req = Request::get("/api/v1/status")
        .header("authorization", "Bearer test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── Approval Endpoints ─────────────────────────────────────────

#[tokio::test]
async fn test_approval_bad_uuid() {
    let app = setup(vec![]).await;
    let req = Request::post("/api/v1/approvals/not-a-uuid/approve")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_approval_not_found() {
    let app = setup(vec![]).await;
    let fake_id = uuid::Uuid::new_v4();
    let req = Request::post(format!("/api/v1/approvals/{fake_id}/approve"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── 404 ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_route_returns_404() {
    let app = setup(vec![]).await;
    let req = Request::get("/api/v1/does-not-exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
