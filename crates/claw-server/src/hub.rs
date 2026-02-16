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
            CREATE INDEX IF NOT EXISTS idx_skills_tags ON skills(tags);

            CREATE TABLE IF NOT EXISTS plugins (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '0.1.0',
                authors TEXT NOT NULL DEFAULT '[]',
                license TEXT NOT NULL DEFAULT '',
                checksum TEXT NOT NULL DEFAULT '',
                tools_json TEXT NOT NULL DEFAULT '[]',
                capabilities_json TEXT NOT NULL DEFAULT '{}',
                wasm_bytes BLOB NOT NULL,
                manifest_toml TEXT NOT NULL,
                downloads INTEGER NOT NULL DEFAULT 0,
                published_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_plugins_name ON plugins(name);",
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
        // Skills
        .route("/api/v1/hub/skills", get(list_skills).post(publish_skill))
        .route("/api/v1/hub/skills/search", get(search_skills))
        .route(
            "/api/v1/hub/skills/{name}",
            get(get_skill).delete(delete_skill),
        )
        .route("/api/v1/hub/skills/{name}/pull", post(pull_skill))
        // Plugins
        .route(
            "/api/v1/hub/plugins",
            get(list_plugins).post(publish_plugin),
        )
        .route("/api/v1/hub/plugins/search", get(search_plugins))
        .route(
            "/api/v1/hub/plugins/{name}",
            get(get_plugin).delete(delete_plugin),
        )
        .route("/api/v1/hub/plugins/{name}/{version}", get(download_plugin))
        // Stats
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
        // Skills
        .route("/api/v1/hub/skills", get(proxy_forward).post(proxy_forward))
        .route("/api/v1/hub/skills/search", get(proxy_forward))
        .route(
            "/api/v1/hub/skills/{name}",
            get(proxy_forward).delete(proxy_forward),
        )
        .route("/api/v1/hub/skills/{name}/pull", post(proxy_forward))
        // Plugins
        .route(
            "/api/v1/hub/plugins",
            get(proxy_forward).post(proxy_forward),
        )
        .route("/api/v1/hub/plugins/search", get(proxy_forward))
        .route(
            "/api/v1/hub/plugins/{name}",
            get(proxy_forward).delete(proxy_forward),
        )
        .route("/api/v1/hub/plugins/{name}/{version}", get(proxy_forward))
        // Stats + Health
        .route("/api/v1/hub/stats", get(proxy_forward))
        .route("/api/v1/hub/health", get(proxy_forward))
}

// Generic proxy handler — forwards the request to the remote hub, streams
// the response back verbatim.

// ═══════════════════════════════════════════════════════════════
// MODE 3 — Local hub (no remote hub configured)
//
// Serves skills from the local skills directory (~/.claw/skills/)
// and plugins from the local plugins directory (~/.claw/plugins/)
// so the web UI's Hub page always shows something useful.
// ═══════════════════════════════════════════════════════════════

/// Build routes that serve local skills + plugins from the filesystem.
/// Merged into the agent's router when NO `services.hub_url` is set.
pub fn local_hub_routes() -> Router<Arc<crate::AppState>> {
    Router::new()
        .route(
            "/api/v1/hub/skills",
            get(local_list_skills).post(local_publish_skill),
        )
        .route("/api/v1/hub/skills/search", get(local_list_skills))
        .route(
            "/api/v1/hub/skills/{name}",
            get(local_get_skill).delete(local_delete_skill),
        )
        .route("/api/v1/hub/skills/{name}/pull", post(local_pull_skill))
        .route("/api/v1/hub/plugins", get(local_list_plugins))
        .route("/api/v1/hub/plugins/search", get(local_list_plugins))
        .route(
            "/api/v1/hub/plugins/{name}",
            get(local_get_plugin).delete(local_delete_plugin),
        )
        .route("/api/v1/hub/stats", get(local_hub_stats))
}

/// List skills from the local filesystem.
async fn local_list_skills(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<SearchParams>,
) -> Json<serde_json::Value> {
    let skills = discover_local_skills(&state.skills_dir);

    let q = params.q.to_lowercase();
    let tag = params.tag.clone().unwrap_or_default().to_lowercase();

    let filtered: Vec<_> = skills
        .into_iter()
        .filter(|s| {
            (q.is_empty()
                || s["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase().contains(&q))
                || s["description"]
                    .as_str()
                    .is_some_and(|d| d.to_lowercase().contains(&q)))
                && (tag.is_empty()
                    || s["tags"].as_array().is_some_and(|tags| {
                        tags.iter()
                            .any(|t| t.as_str().is_some_and(|t| t.to_lowercase() == tag))
                    }))
        })
        .collect();

    let total = filtered.len();

    Json(serde_json::json!({
        "skills": filtered,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))
}

/// Get a single skill by name.
async fn local_get_skill(
    State(state): State<Arc<crate::AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let skills = discover_local_skills(&state.skills_dir);
    skills
        .into_iter()
        .find(|s| s["name"].as_str() == Some(&name))
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// List plugins from the local filesystem.
async fn local_list_plugins(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<SearchParams>,
) -> Json<serde_json::Value> {
    let plugins = discover_local_plugins(&state.plugin_dir);

    let q = params.q.to_lowercase();
    let filtered: Vec<_> = plugins
        .into_iter()
        .filter(|p| {
            q.is_empty()
                || p["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase().contains(&q))
                || p["description"]
                    .as_str()
                    .is_some_and(|d| d.to_lowercase().contains(&q))
        })
        .collect();

    let total = filtered.len();

    Json(serde_json::json!({
        "plugins": filtered,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))
}

/// Get a single plugin by name.
async fn local_get_plugin(
    State(state): State<Arc<crate::AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let plugins = discover_local_plugins(&state.plugin_dir);
    plugins
        .into_iter()
        .find(|p| p["name"].as_str() == Some(name.as_str()))
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// "Pull" a local skill — it's already on disk, so just return success with the version.
async fn local_pull_skill(
    State(state): State<Arc<crate::AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let skills = discover_local_skills(&state.skills_dir);
    let skill = skills
        .into_iter()
        .find(|s| s["name"].as_str() == Some(&name))
        .ok_or(StatusCode::NOT_FOUND)?;

    let version = skill["version"].as_str().unwrap_or("1.0.0");
    Ok(Json(serde_json::json!({
        "name": name,
        "version": version,
        "skill_content": skill["skill_content"],
    })))
}

/// Delete a local skill by removing its directory from disk.
async fn local_delete_skill(
    State(state): State<Arc<crate::AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Find the skill to get its directory name
    let mut registry = claw_skills::SkillRegistry::new_single(&state.skills_dir);
    let _ = registry.discover();

    let def = registry.get(&name).ok_or(StatusCode::NOT_FOUND)?;
    let skill_dir = def.base_dir.clone();

    // Safety: only delete if it's actually inside the skills directory
    if !skill_dir.starts_with(&state.skills_dir) {
        warn!(skill = %name, dir = ?skill_dir, "refusing to delete skill outside skills_dir");
        return Err(StatusCode::FORBIDDEN);
    }

    std::fs::remove_dir_all(&skill_dir).map_err(|e| {
        warn!(error = %e, skill = %name, "failed to delete skill directory");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(skill = %name, "local skill deleted");
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

/// Publish (create) a new skill locally by writing a SKILL.md to the skills directory.
async fn local_publish_skill(
    State(state): State<Arc<crate::AppState>>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Parse to validate
    let def = claw_skills::SkillDefinition::parse(
        &req.skill_content,
        std::path::PathBuf::from("local://uploaded"),
        std::path::PathBuf::from("local://"),
    )
    .map_err(|e| {
        warn!(error = %e, "invalid SKILL.md submitted locally");
        StatusCode::BAD_REQUEST
    })?;

    // Derive a directory-safe name from the skill name
    let dir_name: String = def
        .name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let skill_dir = state.skills_dir.join(&dir_name);

    std::fs::create_dir_all(&skill_dir).map_err(|e| {
        warn!(error = %e, "failed to create skill directory");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    std::fs::write(skill_dir.join("SKILL.md"), &req.skill_content).map_err(|e| {
        warn!(error = %e, "failed to write SKILL.md");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(skill = %def.name, version = %def.version, "skill published locally");
    Ok(Json(serde_json::json!({
        "status": "published",
        "name": def.name,
        "version": def.version,
    })))
}

/// Delete a local plugin by removing its directory from disk.
async fn local_delete_plugin(
    State(state): State<Arc<crate::AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let plugin_dir = state.plugin_dir.join(&name);

    if !plugin_dir.exists() || !plugin_dir.starts_with(&state.plugin_dir) {
        return Err(StatusCode::NOT_FOUND);
    }

    std::fs::remove_dir_all(&plugin_dir).map_err(|e| {
        warn!(error = %e, plugin = %name, "failed to delete plugin directory");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(plugin = %name, "local plugin deleted");
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

/// Hub stats from the local filesystem.
async fn local_hub_stats(State(state): State<Arc<crate::AppState>>) -> Json<serde_json::Value> {
    let skills = discover_local_skills(&state.skills_dir);
    let plugins = discover_local_plugins(&state.plugin_dir);

    // Gather tags
    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for skill in &skills {
        if let Some(tags) = skill["tags"].as_array() {
            for tag in tags {
                if let Some(t) = tag.as_str() {
                    *tag_counts.entry(t.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    let mut top_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    top_tags.sort_by(|a, b| b.1.cmp(&a.1));
    top_tags.truncate(20);

    Json(serde_json::json!({
        "total_skills": skills.len(),
        "total_downloads": 0,
        "total_plugins": plugins.len(),
        "total_plugin_downloads": 0,
        "top_tags": top_tags.iter().map(|(t, c)| serde_json::json!({"tag": t, "count": c})).collect::<Vec<_>>(),
    }))
}

/// Scan the skills directory and return JSON objects matching the HubSkill shape.
fn discover_local_skills(skills_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut registry = claw_skills::SkillRegistry::new_single(skills_dir);
    let _ = registry.discover();

    registry
        .list()
        .into_iter()
        .map(|def| {
            // Count steps: lines starting with a number, "- Step", or "## Step"
            let steps_count = def
                .body
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    t.starts_with("## Step")
                        || t.starts_with("- Step")
                        || t.chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_digit() && t.contains('.'))
                })
                .count()
                .max(1);

            serde_json::json!({
                "id": format!("{}@{}", def.name, def.version),
                "name": def.name,
                "description": def.description,
                "version": def.version,
                "author": def.author.as_deref().unwrap_or(""),
                "tags": def.tags,
                "skill_content": format!("---\nname: {}\ndescription: {}\nversion: {}\ntags: {:?}\n---\n\n{}", def.name, def.description, def.version, def.tags, def.body),
                "downloads": 0,
                "steps_count": steps_count,
                "risk_level": 0,
                "published_at": "",
                "updated_at": "",
            })
        })
        .collect()
}

/// Scan the plugins directory and return JSON objects matching the HubPlugin shape.
fn discover_local_plugins(plugin_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut host = match claw_plugin::PluginHost::new(plugin_dir) {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let _ = host.discover();

    host.loaded()
        .into_iter()
        .map(|manifest| {
            // Compute WASM size by finding the .wasm file
            let wasm_size = find_wasm_size(plugin_dir, &manifest.plugin.name);

            let tools: Vec<serde_json::Value> = manifest
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                    })
                })
                .collect();

            serde_json::json!({
                "id": format!("{}@{}", manifest.plugin.name, manifest.plugin.version),
                "name": manifest.plugin.name,
                "description": manifest.plugin.description,
                "version": manifest.plugin.version,
                "authors": manifest.plugin.authors,
                "license": manifest.plugin.license.as_deref().unwrap_or(""),
                "checksum": manifest.plugin.checksum.as_deref().unwrap_or(""),
                "tools": tools,
                "capabilities": serde_json::json!({
                    "network": manifest.capabilities.network,
                    "filesystem": manifest.capabilities.filesystem,
                    "shell": manifest.capabilities.shell,
                }),
                "wasm_size": wasm_size,
                "downloads": 0,
                "published_at": "",
                "updated_at": "",
            })
        })
        .collect()
}

/// Find the size of the .wasm file in a plugin directory.
fn find_wasm_size(plugin_dir: &std::path::Path, name: &str) -> u64 {
    let dir = plugin_dir.join(name);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "wasm") {
                return entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    0
}
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
        .query_map(
            rusqlite::params![params.limit as i64, params.offset as i64],
            |row| Ok(row_to_skill(row)),
        )
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

    let total_plugins: i64 = db
        .query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0))
        .unwrap_or(0);

    let total_plugin_downloads: i64 = db
        .query_row("SELECT COALESCE(SUM(downloads), 0) FROM plugins", [], |r| {
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
        "total_plugins": total_plugins,
        "total_plugin_downloads": total_plugin_downloads,
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

// ═══════════════════════════════════════════════════════════════
// Plugin Hub — WASM plugin registry
// ═══════════════════════════════════════════════════════════════

/// A plugin published to the hub (metadata only — WASM blob excluded for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub authors: Vec<String>,
    pub license: String,
    pub checksum: String,
    pub tools: Vec<HubPluginTool>,
    pub capabilities: serde_json::Value,
    pub downloads: u64,
    pub wasm_size: u64,
    pub published_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubPluginTool {
    pub name: String,
    pub description: String,
    pub risk_level: u8,
    pub is_mutating: bool,
}

async fn list_plugins(
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
        "SELECT id, name, description, version, authors, license, checksum, \
         tools_json, capabilities_json, length(wasm_bytes), downloads, published_at, updated_at \
         FROM plugins {sort_clause} LIMIT ?1 OFFSET ?2"
    );

    let mut stmt = db
        .prepare(&sql)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let plugins: Vec<HubPlugin> = stmt
        .query_map(
            rusqlite::params![params.limit as i64, params.offset as i64],
            |row| Ok(row_to_plugin(row)),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0))
        .unwrap_or(0);

    Ok(Json(serde_json::json!({
        "plugins": plugins,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    })))
}

async fn search_plugins(
    State(hub): State<Arc<HubState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let sql = "SELECT id, name, description, version, authors, license, checksum, \
               tools_json, capabilities_json, length(wasm_bytes), downloads, published_at, updated_at \
               FROM plugins WHERE name LIKE ?1 OR description LIKE ?1 \
               ORDER BY downloads DESC LIMIT ?2 OFFSET ?3";

    let pattern = format!("%{}%", params.q);
    let mut stmt = db
        .prepare(sql)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let plugins: Vec<HubPlugin> = stmt
        .query_map(
            rusqlite::params![pattern, params.limit as i64, params.offset as i64],
            |row| Ok(row_to_plugin(row)),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(serde_json::json!({
        "plugins": plugins,
        "query": params.q,
    })))
}

async fn get_plugin(
    State(hub): State<Arc<HubState>>,
    Path(name): Path<String>,
) -> Result<Json<HubPlugin>, StatusCode> {
    let db = hub.db.lock().await;

    let plugin = db
        .query_row(
            "SELECT id, name, description, version, authors, license, checksum, \
             tools_json, capabilities_json, length(wasm_bytes), downloads, published_at, updated_at \
             FROM plugins WHERE name = ?1",
            [&name],
            |row| Ok(row_to_plugin(row)),
        )
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(plugin))
}

/// Download the WASM binary for a plugin.
/// GET /api/v1/hub/plugins/{name}/{version}
/// Returns the raw .wasm bytes (application/wasm) for `claw plugin install`.
async fn download_plugin(
    State(hub): State<Arc<HubState>>,
    Path((name, _version)): Path<(String, String)>,
) -> Result<Response<Body>, StatusCode> {
    let db = hub.db.lock().await;

    let result: Result<(Vec<u8>, String), _> = db.query_row(
        "SELECT wasm_bytes, manifest_toml FROM plugins WHERE name = ?1",
        [&name],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
    );

    match result {
        Ok((wasm_bytes, _manifest)) => {
            let _ = db.execute(
                "UPDATE plugins SET downloads = downloads + 1 WHERE name = ?1",
                [&name],
            );
            info!(plugin = %name, "plugin downloaded from hub");

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/wasm")
                .header(
                    "Content-Disposition",
                    format!("attachment; filename=\"{name}.wasm\""),
                )
                .body(Body::from(wasm_bytes))
                .unwrap())
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

/// Publish a plugin — multipart form with `manifest` (TOML text) + `wasm` (binary).
/// If multipart isn't convenient, also accepts JSON with base64-encoded WASM.
async fn publish_plugin(
    State(hub): State<Arc<HubState>>,
    Json(req): Json<PublishPluginRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Parse the manifest
    let manifest: claw_plugin::PluginManifest =
        toml::from_str(&req.manifest_toml).map_err(|e| {
            warn!(error = %e, "invalid plugin.toml submitted to hub");
            StatusCode::BAD_REQUEST
        })?;

    // Decode base64 WASM bytes
    use base64::Engine as _;
    let wasm_bytes = base64::engine::general_purpose::STANDARD
        .decode(&req.wasm_base64)
        .map_err(|e| {
            warn!(error = %e, "invalid base64 WASM data");
            StatusCode::BAD_REQUEST
        })?;

    // Verify checksum if provided
    if !manifest.verify_checksum(&wasm_bytes) {
        warn!(plugin = %manifest.plugin.name, "checksum mismatch on upload");
        return Err(StatusCode::BAD_REQUEST);
    }

    let name = &manifest.plugin.name;
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}@{}", name, manifest.plugin.version);
    let checksum = blake3::hash(&wasm_bytes).to_hex().to_string();
    let authors_json =
        serde_json::to_string(&manifest.plugin.authors).unwrap_or_else(|_| "[]".into());
    let tools_json = serde_json::to_string(&manifest.tools).unwrap_or_else(|_| "[]".into());
    let caps_json = serde_json::to_string(&manifest.capabilities).unwrap_or_else(|_| "{}".into());

    let db = hub.db.lock().await;

    db.execute(
        "INSERT INTO plugins (id, name, description, version, authors, license, checksum, \
         tools_json, capabilities_json, wasm_bytes, manifest_toml, downloads, published_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?12)
         ON CONFLICT(name) DO UPDATE SET
            id = ?1, description = ?3, version = ?4, authors = ?5, license = ?6,
            checksum = ?7, tools_json = ?8, capabilities_json = ?9,
            wasm_bytes = ?10, manifest_toml = ?11, updated_at = ?12",
        rusqlite::params![
            id,
            name,
            manifest.plugin.description,
            manifest.plugin.version,
            authors_json,
            manifest.plugin.license.as_deref().unwrap_or(""),
            checksum,
            tools_json,
            caps_json,
            wasm_bytes,
            req.manifest_toml,
            now,
        ],
    )
    .map_err(|e| {
        warn!(error = %e, "failed to publish plugin");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(plugin = %name, version = %manifest.plugin.version, wasm_size = wasm_bytes.len(), "plugin published to hub");

    Ok(Json(serde_json::json!({
        "status": "published",
        "name": name,
        "version": manifest.plugin.version,
        "checksum": checksum,
        "wasm_size": wasm_bytes.len(),
    })))
}

async fn delete_plugin(
    State(hub): State<Arc<HubState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = hub.db.lock().await;

    let affected = db
        .execute("DELETE FROM plugins WHERE name = ?1", [&name])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if affected == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    info!(plugin = %name, "plugin deleted from hub");

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

#[derive(Deserialize)]
pub struct PublishPluginRequest {
    /// The plugin.toml manifest content
    pub manifest_toml: String,
    /// Base64-encoded WASM binary
    pub wasm_base64: String,
}

fn row_to_plugin(row: &rusqlite::Row) -> HubPlugin {
    let authors_str: String = row.get(4).unwrap_or_default();
    let authors: Vec<String> = serde_json::from_str(&authors_str).unwrap_or_default();
    let tools_str: String = row.get(7).unwrap_or_default();
    let tools: Vec<HubPluginTool> = serde_json::from_str(&tools_str).unwrap_or_default();
    let caps_str: String = row.get(8).unwrap_or_default();
    let capabilities: serde_json::Value = serde_json::from_str(&caps_str).unwrap_or_default();

    HubPlugin {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        description: row.get(2).unwrap_or_default(),
        version: row.get(3).unwrap_or_default(),
        authors,
        license: row.get(5).unwrap_or_default(),
        checksum: row.get(6).unwrap_or_default(),
        tools,
        capabilities,
        wasm_size: row.get::<_, i64>(9).unwrap_or(0) as u64,
        downloads: row.get::<_, i64>(10).unwrap_or(0) as u64,
        published_at: row.get(11).unwrap_or_default(),
        updated_at: row.get(12).unwrap_or_default(),
    }
}
