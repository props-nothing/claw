//! # Skills Hub
//!
//! A centralised skill marketplace that **any** Claw agent can connect to from
//! anywhere on the network. Two operating modes:
//!
//! 1. **Standalone hub server** — run `claw hub serve` to host a hub that
//!    remote agents publish to / pull from.
//! 2. **Proxy mode** — when the agent has `services.hub_url` configured the
//!    local web UI proxies through `/api/v1/hub/*` so the browser never needs
//!    to deal with CORS to the remote hub.

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{Json, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

// ── Data types ────────────────────────────────────────────────

/// A skill published to the hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub tags: Vec<String>,
    /// Markdown content of the SKILL.md
    pub skill_content: String,
    pub downloads: u64,
    pub published_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct PublishRequest {
    /// SKILL.md content (Markdown with YAML frontmatter)
    pub skill_content: String,
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub sort: Option<String>,
}

fn default_limit() -> usize {
    50
}

// ── Hub State (used by the standalone server) ─────────────────

pub struct HubState {
    pub db: Mutex<rusqlite::Connection>,
}

impl HubState {
    pub fn new(db_path: &std::path::Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| format!("failed to open hub db: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '1.0.0',
                author TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '[]',
                skill_content TEXT NOT NULL,
                downloads INTEGER NOT NULL DEFAULT 0,
                published_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_skills_name ON skills(name);
            CREATE INDEX IF NOT EXISTS idx_skills_tags ON skills(tags);",
        )
        .map_err(|e| format!("failed to create hub tables: {e}"))?;

        Ok(Self {
            db: Mutex::new(conn),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// MODE 1 — Standalone Hub Server (`claw hub serve`)
// ═══════════════════════════════════════════════════════════════

/// Build a fully self-contained hub router with its own state and CORS.
/// Called by `claw hub serve` — no dependency on `AppState`.
pub fn standalone_hub_router(db_path: &std::path::Path) -> Result<Router, String> {
    let state = Arc::new(HubState::new(db_path)?);

    let router = Router::new()
        .route("/health", get(hub_health))
        .route("/api/v1/hub/skills", get(list_skills).post(publish_skill))
        .route("/api/v1/hub/skills/search", get(search_skills))
        .route(
            "/api/v1/hub/skills/{name}",
            get(get_skill).delete(delete_skill),
        )
        .route("/api/v1/hub/skills/{name}/pull", post(pull_skill))
        .route("/api/v1/hub/stats", get(hub_stats))
        .with_state(state)
        .layer(CorsLayer::permissive());

    Ok(router)
}

async fn hub_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "claw-skills-hub",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ═══════════════════════════════════════════════════════════════
// MODE 2 — Proxy routes (agent forwards to remote hub)
// ═══════════════════════════════════════════════════════════════

/// Proxy state — holds the remote hub URL and an HTTP client.
pub struct HubProxy {
    pub hub_url: String,
    pub client: reqwest::Client,
}

/// Build proxy routes that forward `/api/v1/hub/*` to the remote hub.
/// Merged into the agent's router when `services.hub_url` is set.
pub fn hub_proxy_routes() -> Router<Arc<crate::AppState>> {
    Router::new()
        .route("/api/v1/hub/skills", get(proxy_forward).post(proxy_forward))
        .route("/api/v1/hub/skills/search", get(proxy_forward))
        .route(
            "/api/v1/hub/skills/{name}",
            get(proxy_forward).delete(proxy_forward),
        )
        .route("/api/v1/hub/skills/{name}/pull", post(proxy_forward))
        .route("/api/v1/hub/stats", get(proxy_forward))
        .route("/api/v1/hub/health", get(proxy_forward))
}

/// Generic proxy handler — forwards the request to the remote hub, streams
/// the response back verbatim.
async fn proxy_forward(
    State(state): State<Arc<crate::AppState>>,
    req: Request,
) -> Result<Response<Body>, StatusCode> {
    let proxy = state
        .hub_proxy
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let hub_base = proxy.hub_url.trim_end_matches('/');
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.uri().path());

    let target_url = format!("{hub_base}{path_and_query}");

    // Map method
    let method = match req.method().clone() {
        axum::http::Method::GET => reqwest::Method::GET,
        axum::http::Method::POST => reqwest::Method::POST,
        axum::http::Method::PUT => reqwest::Method::PUT,
        axum::http::Method::DELETE => reqwest::Method::DELETE,
        _ => reqwest::Method::GET,
    };

    // Forward content-type header if present
    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Read the body
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut upstream_req = proxy.client.request(method, &target_url);
    if let Some(ct) = content_type {
        upstream_req = upstream_req.header("Content-Type", ct);
    }
    if !body_bytes.is_empty() {
        upstream_req = upstream_req.body(body_bytes);
    }

    let upstream_resp = upstream_req.send().await.map_err(|e| {
        warn!(error = %e, url = %target_url, "hub proxy request failed");
        StatusCode::BAD_GATEWAY
    })?;

    // Build the response back to the browser
    let status = StatusCode::from_u16(upstream_resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let resp_bytes = upstream_resp
        .bytes()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()
        }))
}

// ═══════════════════════════════════════════════════════════════
// Handlers (used by standalone hub server)
// ═══════════════════════════════════════════════════════════════

async fn list_skills(
    State(hub): State<Arc<HubState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let sort_clause = match params.sort.as_deref() {
        Some("downloads") => "ORDER BY downloads DESC",
        Some("name") => "ORDER BY name ASC",
        Some("updated") => "ORDER BY updated_at DESC",
        _ => "ORDER BY updated_at DESC",
    };

    let sql = format!(
        "SELECT id, name, description, version, author, tags, \
         skill_content, downloads, published_at, updated_at \
         FROM skills {sort_clause} LIMIT ?1 OFFSET ?2"
    );

    let mut stmt = db
        .prepare(&sql)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let skills: Vec<HubSkill> = stmt
        .query_map(rusqlite::params![params.limit, params.offset], |row| {
            Ok(row_to_skill(row))
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .unwrap_or(0);

    Ok(Json(serde_json::json!({
        "skills": skills,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    })))
}

async fn search_skills(
    State(hub): State<Arc<HubState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let mut conditions = Vec::new();
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if !params.q.is_empty() {
        conditions.push("(name LIKE ?1 OR description LIKE ?1 OR tags LIKE ?1)".to_string());
        bind_values.push(Box::new(format!("%{}%", params.q)));
    }

    if let Some(ref tag) = params.tag {
        let idx = bind_values.len() + 1;
        conditions.push(format!("tags LIKE ?{idx}"));
        bind_values.push(Box::new(format!("%\"{tag}%")));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, name, description, version, author, tags, \
         skill_content, downloads, published_at, updated_at \
         FROM skills {} ORDER BY downloads DESC LIMIT ?{} OFFSET ?{}",
        where_clause,
        bind_values.len() + 1,
        bind_values.len() + 2,
    );

    bind_values.push(Box::new(params.limit as i64));
    bind_values.push(Box::new(params.offset as i64));

    let refs: Vec<&dyn rusqlite::types::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();

    let mut stmt = db
        .prepare(&sql)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let skills: Vec<HubSkill> = stmt
        .query_map(refs.as_slice(), |row| Ok(row_to_skill(row)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(serde_json::json!({
        "skills": skills,
        "query": params.q,
        "tag": params.tag,
    })))
}

async fn publish_skill(
    State(hub): State<Arc<HubState>>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Parse SKILL.md content
    let def = claw_skills::SkillDefinition::parse(
        &req.skill_content,
        std::path::PathBuf::from("hub://uploaded"),
        std::path::PathBuf::from("hub://"),
    )
    .map_err(|e| {
        warn!(error = %e, "invalid SKILL.md submitted to hub");
        StatusCode::BAD_REQUEST
    })?;

    let name = &def.name;
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}@{}", name, def.version);
    let tags_json = serde_json::to_string(&def.tags).unwrap_or_else(|_| "[]".into());

    let db = hub.db.lock().await;

    db.execute(
        "INSERT INTO skills (id, name, description, version, author, tags, \
         skill_content, downloads, published_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?8)
         ON CONFLICT(name) DO UPDATE SET
            id = ?1, description = ?3, version = ?4, author = ?5, tags = ?6,
            skill_content = ?7, updated_at = ?8",
        rusqlite::params![
            id,
            name,
            def.description,
            def.version,
            def.author.as_deref().unwrap_or(""),
            tags_json,
            req.skill_content,
            now,
        ],
    )
    .map_err(|e| {
        warn!(error = %e, "failed to publish skill");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(skill = %name, version = %def.version, "skill published to hub");

    Ok(Json(serde_json::json!({
        "status": "published",
        "name": name,
        "version": def.version,
    })))
}

async fn get_skill(
    State(hub): State<Arc<HubState>>,
    Path(name): Path<String>,
) -> Result<Json<HubSkill>, StatusCode> {
    let db = hub.db.lock().await;

    let skill = db
        .query_row(
            "SELECT id, name, description, version, author, tags, \
             skill_content, downloads, published_at, updated_at \
             FROM skills WHERE name = ?1",
            [&name],
            |row| Ok(row_to_skill(row)),
        )
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(skill))
}

async fn pull_skill(
    State(hub): State<Arc<HubState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let result: Result<(String, String, String), _> = db.query_row(
        "SELECT name, version, skill_content FROM skills WHERE name = ?1",
        [&name],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    );

    match result {
        Ok((skill_name, version, skill_content)) => {
            let _ = db.execute(
                "UPDATE skills SET downloads = downloads + 1 WHERE name = ?1",
                [&name],
            );
            info!(skill = %skill_name, "skill pulled from hub");
            Ok(Json(serde_json::json!({
                "name": skill_name,
                "version": version,
                "skill_content": skill_content,
            })))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn delete_skill(
    State(hub): State<Arc<HubState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let affected = db
        .execute("DELETE FROM skills WHERE name = ?1", [&name])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if affected == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    info!(skill = %name, "skill deleted from hub");

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

async fn hub_stats(
    State(hub): State<Arc<HubState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let total_skills: i64 = db
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .unwrap_or(0);

    let total_downloads: i64 = db
        .query_row("SELECT COALESCE(SUM(downloads), 0) FROM skills", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);

    let mut stmt = db
        .prepare("SELECT tags FROM skills")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for row in rows.flatten() {
        if let Ok(tags) = serde_json::from_str::<Vec<String>>(&row) {
            for tag in tags {
                *tag_counts.entry(tag).or_insert(0) += 1;
            }
        }
    }

    let mut top_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    top_tags.sort_by(|a, b| b.1.cmp(&a.1));
    top_tags.truncate(20);

    Ok(Json(serde_json::json!({
        "total_skills": total_skills,
        "total_downloads": total_downloads,
        "top_tags": top_tags.iter().map(|(t, c)| serde_json::json!({"tag": t, "count": c})).collect::<Vec<_>>(),
    })))
}

// ── Helpers ───────────────────────────────────────────────────

fn row_to_skill(row: &rusqlite::Row) -> HubSkill {
    let tags_str: String = row.get(5).unwrap_or_default();
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();

    HubSkill {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        description: row.get(2).unwrap_or_default(),
        version: row.get(3).unwrap_or_default(),
        author: row.get(4).unwrap_or_default(),
        tags,
        skill_content: row.get(6).unwrap_or_default(),
        downloads: row.get::<_, i64>(7).unwrap_or(0) as u64,
        published_at: row.get(8).unwrap_or_default(),
        updated_at: row.get(9).unwrap_or_default(),
    }
}
