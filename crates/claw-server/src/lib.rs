//! # claw-server
//!
//! HTTP/WebSocket API server for the Claw runtime. Provides:
//!
//! - REST API for sending messages, managing sessions, viewing goals
//! - WebSocket endpoint for real-time streaming
//! - Webhook receiver for external triggers
//! - Web UI static file serving

pub mod hub;
pub mod metrics;
pub mod ratelimit;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{Json, Response, Sse, sse::Event as SseEvent},
    routing::{get, post},
};
use claw_config::schema::ServerConfig;
use claw_runtime::{QueryKind, RuntimeHandle, StreamEvent, get_runtime_handle};
use futures::stream::Stream;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::{info, warn};

/// Web UI assets embedded at compile time from the workspace `web/` directory.
#[derive(RustEmbed)]
#[folder = "../../web/"]
struct WebAssets;

/// Shared server state.
pub struct AppState {
    pub config: ServerConfig,
    /// Lazily populated once the runtime is up.
    pub handle: RwLock<Option<RuntimeHandle>>,
    /// Prometheus-compatible metrics.
    pub metrics: metrics::Metrics,
    /// Hub proxy — forwards `/api/v1/hub/*` to the remote Skills Hub when
    /// `services.hub_url` is configured.
    pub hub_proxy: Option<hub::HubProxy>,
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    uptime_secs: u64,
}

/// Chat request body.
#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    session_id: Option<String>,
}

/// Chat response body.
#[derive(Serialize)]
struct ChatResponse {
    response: String,
    session_id: String,
}

/// Query params for memory search.
#[derive(Deserialize)]
struct MemorySearchParams {
    q: String,
}

/// Query params for audit log.
#[derive(Deserialize)]
struct AuditLogParams {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

/// Build the Axum router.
pub fn build_router(config: ServerConfig, hub_url: Option<String>) -> Router {
    // Set up hub proxy if a remote hub URL is configured
    let hub_proxy = hub_url.map(|url| {
        info!(hub_url = %url, "skills hub proxy enabled — forwarding /api/v1/hub/* to remote hub");
        hub::HubProxy {
            hub_url: url,
            client: reqwest::Client::new(),
        }
    });

    let state = Arc::new(AppState {
        config: config.clone(),
        handle: RwLock::new(None),
        metrics: metrics::Metrics::new(),
        hub_proxy,
    });

    let api_routes = Router::new()
        .route("/api/v1/chat", post(chat_handler))
        .route("/api/v1/chat/stream", post(chat_stream_handler))
        .route("/api/v1/sessions", get(sessions_handler))
        .route(
            "/api/v1/sessions/{id}/messages",
            get(session_messages_handler),
        )
        .route("/api/v1/goals", get(goals_handler))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/tools", get(tools_handler))
        .route("/api/v1/memory/facts", get(facts_handler))
        .route("/api/v1/memory/search", get(memory_search_handler))
        .route("/api/v1/config", get(config_handler))
        .route("/api/v1/audit", get(audit_handler))
        .route(
            "/api/v1/approvals/{id}/approve",
            post(approval_approve_handler),
        )
        .route("/api/v1/approvals/{id}/deny", post(approval_deny_handler))
        .route("/api/v1/mesh/status", get(mesh_status_handler))
        .route("/api/v1/mesh/peers", get(mesh_peers_handler))
        .route("/api/v1/mesh/send", post(mesh_send_handler));

    // Apply API key auth if configured
    let api_routes = if config.api_key.is_some() {
        api_routes.layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
    } else {
        api_routes
    };

    // Apply rate limiting to API routes
    // Note: layers execute outermost-first. The Extension layer must wrap the
    // middleware so the RateLimiter is present in the request extensions when
    // rate_limit_middleware tries to extract it.
    let rate_limiter = ratelimit::RateLimiter::new(ratelimit::RateLimitConfig::default());
    let api_routes = api_routes
        .layer(middleware::from_fn(ratelimit::rate_limit_middleware))
        .layer(axum::Extension(rate_limiter.clone()));

    // Spawn background cleanup task for stale rate-limit buckets
    tokio::spawn({
        let limiter = rate_limiter;
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                limiter.cleanup();
            }
        }
    });

    // Determine the web directory for static file serving
    let web_dir = find_web_dir();

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        // Screenshot serving is outside auth — <img> tags can't send auth headers
        .route("/api/v1/screenshots/{filename}", get(screenshot_handler))
        .merge(api_routes);

    // Merge hub proxy routes when a remote hub URL is configured
    if state.hub_proxy.is_some() {
        router = router.merge(hub::hub_proxy_routes());
    }

    // Serve static files for the Web UI
    if config.web_ui {
        if let Some(ref dir) = web_dir {
            info!(path = %dir.display(), "serving web UI from filesystem (dev override)");
            router = router.fallback_service(ServeDir::new(dir));
        } else {
            info!("serving web UI from embedded assets");
            router = router.fallback_service(get(embedded_file_handler));
        }
    }

    let mut router = router.with_state(state.clone());

    if config.cors {
        router = router.layer(CorsLayer::permissive());
    }

    router
}

/// Find the web/ directory — checks CWD first (for development), then next to the binary,
/// then ~/.claw/web as a last resort.
fn find_web_dir() -> Option<std::path::PathBuf> {
    // 1. Check current working directory (best for development)
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("web");
        if p.join("index.html").exists() {
            return Some(p);
        }
    }
    // 2. Check next to the binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("web");
            if p.join("index.html").exists() {
                return Some(p);
            }
        }
    }
    // 3. Check ~/.claw/web/ (installed fallback)
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".claw").join("web");
        if p.join("index.html").exists() {
            return Some(p);
        }
    }
    None
}

/// Serve a file from the embedded [`WebAssets`].
async fn embedded_file_handler(req: Request<axum::body::Body>) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    // Default to index.html for the root path
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(content) => {
            let mime = content.metadata.mimetype();
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime)
                .body(axum::body::Body::from(content.data.to_vec()))
                .unwrap()
        }
        None => {
            // SPA fallback — serve index.html for unmatched routes
            match WebAssets::get("index.html") {
                Some(content) => {
                    let mime = content.metadata.mimetype();
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", mime)
                        .body(axum::body::Body::from(content.data.to_vec()))
                        .unwrap()
                }
                None => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(axum::body::Body::from("not found"))
                    .unwrap(),
            }
        }
    }
}

/// Middleware that checks the Authorization header against the configured API key.
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(ref expected_key) = state.config.api_key {
        let provided = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(key) if key == expected_key => {}
            _ => {
                warn!("unauthorized API request — invalid or missing API key");
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }
    Ok(next.run(request).await)
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    state.metrics.inc_http_requests();
    // Try to get real uptime from the runtime
    let uptime = if let Some(handle) = get_handle(&state).await {
        match handle.query(QueryKind::Status).await {
            Ok(data) => data["uptime_secs"].as_u64().unwrap_or(0),
            Err(_) => 0,
        }
    } else {
        0
    };
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        uptime_secs: uptime,
    })
}

/// Prometheus-compatible metrics endpoint.
async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let body = state.metrics.render_prometheus();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    state.metrics.inc_http_requests();
    state.metrics.inc_chat_messages();
    // Try to get the runtime handle — it's set by the runtime after startup
    let handle = {
        let guard = state.handle.read().await;
        guard.clone()
    };

    // If we don't have it cached yet, try to fetch from the global
    let handle = match handle {
        Some(h) => h,
        None => {
            match get_runtime_handle().await {
                Some(h) => {
                    // Cache it in our state
                    let mut guard = state.handle.write().await;
                    *guard = Some(h.clone());
                    h
                }
                None => {
                    warn!("chat request received but runtime is not yet ready");
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
            }
        }
    };

    // Send the message to the agent runtime and wait for a response
    match handle.chat(req.message, req.session_id).await {
        Ok(response) => {
            if let Some(err) = response.error {
                warn!(error = %err, "agent returned error");
                Ok(Json(ChatResponse {
                    response: format!("Error: {}", err),
                    session_id: response.session_id,
                }))
            } else {
                Ok(Json(ChatResponse {
                    response: response.text,
                    session_id: response.session_id,
                }))
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to reach agent runtime");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// SSE streaming chat handler — streams tokens back as Server-Sent Events.
async fn chat_stream_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, StatusCode> {
    state.metrics.inc_http_requests();
    state.metrics.inc_chat_stream_messages();
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let mut chunk_rx = handle
        .chat_stream(req.message, req.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let stream = async_stream::stream! {
        while let Some(event) = chunk_rx.recv().await {
            let data = serde_json::to_string(&event).unwrap_or_default();
            yield Ok(SseEvent::default().data(data));
            // If it's a Done or Error event, stop the stream
            if matches!(event, StreamEvent::Done | StreamEvent::Error { .. }) {
                break;
            }
        }
    };

    Ok(Sse::new(stream))
}

async fn sessions_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Sessions).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn session_messages_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::SessionMessages(session_id)).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn goals_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Goals).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Status).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn tools_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Tools).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn facts_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Facts).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn memory_search_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemorySearchParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::MemorySearch(params.q)).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn config_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::Config).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn audit_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditLogParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::AuditLog(params.limit)).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn approval_approve_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let uuid = id
        .parse::<uuid::Uuid>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    match handle.approve(uuid).await {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "approved", "id": id }))),
        Err(e) => {
            warn!(error = %e, "approval not found");
            Err(StatusCode::NOT_FOUND)
        }
    }
}

async fn approval_deny_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let uuid = id
        .parse::<uuid::Uuid>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    match handle.deny(uuid).await {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "denied", "id": id }))),
        Err(e) => {
            warn!(error = %e, "approval not found");
            Err(StatusCode::NOT_FOUND)
        }
    }
}

// ── Mesh endpoints ─────────────────────────────────────────────────────────

async fn mesh_status_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::MeshStatus).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn mesh_peers_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match handle.query(QueryKind::MeshPeers).await {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Request body for sending a mesh message.
#[derive(Deserialize)]
struct MeshSendRequest {
    peer_id: String,
    message: String,
}

async fn mesh_send_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MeshSendRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let handle = get_handle(&state)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Get the runtime's shared state to access the mesh node
    match handle.query(QueryKind::MeshStatus).await {
        Ok(status) => {
            if !status["running"].as_bool().unwrap_or(false) {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
        }
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    }

    // Use the RuntimeHandle's state to send the mesh message directly
    let state_inner = handle.state();
    let mesh = state_inner.mesh.lock().await;
    let our_peer_id = mesh.peer_id().to_string();
    let timestamp = chrono::Utc::now().timestamp();
    let msg = claw_mesh::MeshMessage::DirectMessage {
        from_peer: our_peer_id,
        to_peer: body.peer_id.clone(),
        content: body.message.clone(),
        timestamp,
    };
    match mesh.send_to(&body.peer_id, &msg).await {
        Ok(()) => Ok(Json(serde_json::json!({
            "status": "sent",
            "peer_id": body.peer_id,
        }))),
        Err(e) => {
            warn!(error = %e, "failed to send mesh message");
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// Serve a screenshot file from ~/.claw/screenshots/.
/// Accessed via GET /api/v1/screenshots/{filename}
async fn screenshot_handler(Path(filename): Path<String>) -> Result<Response, StatusCode> {
    use axum::body::Body;
    use axum::http::header;

    // Validate filename: must be alphanumeric + underscores + dots + hyphens, ending in .png
    if !filename.ends_with(".png")
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let screenshots_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".claw")
        .join("screenshots");

    let filepath = screenshots_dir.join(&filename);

    if !filepath.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let bytes = tokio::fs::read(&filepath)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from(bytes))
        .unwrap())
}

/// Helper — get or cache the runtime handle.
async fn get_handle(state: &AppState) -> Option<RuntimeHandle> {
    let guard = state.handle.read().await;
    if let Some(ref h) = *guard {
        return Some(h.clone());
    }
    drop(guard);

    let h = get_runtime_handle().await?;
    let mut guard = state.handle.write().await;
    *guard = Some(h.clone());
    Some(h)
}

/// Start the HTTP server.
pub async fn start_server(config: ServerConfig, hub_url: Option<String>) -> claw_core::Result<()> {
    let listen = config.listen.clone();
    let router = build_router(config, hub_url);

    info!(listen = %listen, "starting HTTP server");

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .map_err(|e| claw_core::ClawError::Agent(format!("failed to bind {}: {}", listen, e)))?;

    axum::serve(listener, router)
        .await
        .map_err(|e| claw_core::ClawError::Agent(format!("server error: {}", e)))?;

    Ok(())
}
