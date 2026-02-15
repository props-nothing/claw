use crate::agent::SharedAgentState;
use claw_device::DeviceTools;
use uuid::Uuid;

/// Kinds of queries the server can ask the runtime.
pub enum QueryKind {
    Status,
    Sessions,
    SessionMessages(String),
    Goals,
    Tools,
    Facts,
    MemorySearch(String),
    Config,
    AuditLog(usize),
    MeshPeers,
    MeshStatus,
    SubTasks,
    ScheduledTasks,
}

pub(crate) async fn handle_query(
    state: &SharedAgentState,
    kind: QueryKind,
    started_at: std::time::Instant,
) -> Result<serde_json::Value, String> {
    let result = match kind {
        QueryKind::Status => {
            let budget = state.budget.snapshot();
            let session_count = state.sessions.active_count().await;
            let channels = state.channels.lock().await;
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "status": "running",
                "uptime_secs": started_at.elapsed().as_secs(),
                "model": &state.config.agent.model,
                "autonomy_level": state.config.autonomy.level,
                "budget": {
                    "spent_usd": budget.daily_spend_usd,
                    "daily_limit_usd": budget.daily_limit_usd,
                    "total_spend_usd": budget.total_spend_usd,
                    "total_tool_calls": budget.total_tool_calls,
                },
                "sessions": session_count,
                "channels": channels.iter().map(|c| c.id().to_string()).collect::<Vec<_>>(),
            })
        }
        QueryKind::Sessions => {
            let sessions = state.sessions.list_sessions().await;
            let list: Vec<serde_json::Value> = sessions
                .iter()
                .filter(|s| s.message_count > 0)
                .map(|s| {
                    serde_json::json!({
                        "id": s.id.to_string(),
                        "name": s.name,
                        "active": s.active,
                        "message_count": s.message_count,
                        "channel": s.channel,
                        "created_at": s.created_at.to_rfc3339(),
                    })
                })
                .collect();
            serde_json::json!({ "sessions": list })
        }
        QueryKind::SessionMessages(ref session_id_str) => {
            if let Ok(sid) = session_id_str.parse::<Uuid>() {
                let mem = state.memory.lock().await;
                let mut messages_slice = mem.working.messages(sid);

                // If working memory is empty, try loading from SQLite
                let persisted;
                if messages_slice.is_empty()
                    && let Ok(msgs) = mem.load_session_messages(&sid)
                    && !msgs.is_empty()
                {
                    persisted = msgs;
                    messages_slice = &persisted;
                }

                let list: Vec<serde_json::Value> = messages_slice
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id.to_string(),
                            "role": m.role,
                            "content": m.text_content(),
                            "tool_calls": m.tool_calls.iter().map(|tc| serde_json::json!({
                                "id": tc.id,
                                "tool_name": tc.tool_name,
                                "arguments": tc.arguments,
                            })).collect::<Vec<_>>(),
                            "timestamp": m.timestamp.to_rfc3339(),
                        })
                    })
                    .collect();
                serde_json::json!({ "messages": list })
            } else {
                serde_json::json!({ "error": "invalid session_id" })
            }
        }
        QueryKind::Goals => {
            let planner = state.planner.lock().await;
            let goals: Vec<serde_json::Value> = planner
                .all()
                .iter()
                .map(|g| {
                    serde_json::json!({
                        "id": g.id.to_string(),
                        "title": g.description,
                        "description": g.description,
                        "status": format!("{:?}", g.status),
                        "priority": g.priority,
                        "progress": g.progress,
                        "created_at": g.created_at.to_rfc3339(),
                        "updated_at": g.updated_at.to_rfc3339(),
                        "steps": g.steps.iter().map(|s| serde_json::json!({
                            "id": s.id.to_string(),
                            "description": s.description,
                            "status": format!("{:?}", s.status),
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            serde_json::json!({ "goals": goals })
        }
        QueryKind::Tools => {
            let tools: Vec<serde_json::Value> = state
                .tools
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": t.provider,
                    })
                })
                .collect();
            let mut plugin_tools: Vec<serde_json::Value> = state
                .plugins
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": t.provider,
                    })
                })
                .collect();
            let mut all = tools;
            all.append(&mut plugin_tools);
            // Skills are prompt-injected (SKILL.md), not listed as tools.
            // They appear in the system prompt via <available_skills>.
            // Add device tools (browser, android, ios)
            let device_tools: Vec<serde_json::Value> = DeviceTools::tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": "device",
                    })
                })
                .collect();
            all.extend(device_tools);
            serde_json::json!({ "tools": all })
        }
        QueryKind::Facts => {
            let mem = state.memory.lock().await;
            let facts: Vec<serde_json::Value> = mem
                .semantic
                .all_facts()
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "id": f.id.to_string(),
                        "category": f.category,
                        "key": f.key,
                        "value": f.value,
                        "confidence": f.confidence,
                        "source": f.source,
                        "created_at": f.created_at.to_rfc3339(),
                        "updated_at": f.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            let count = facts.len();
            serde_json::json!({ "facts": facts, "count": count })
        }
        QueryKind::MemorySearch(ref query_text) => {
            // Embed query for vector search if embedder is available
            let query_embedding = if let Some(ref embedder) = state.embedder {
                match embedder.embed(&[query_text.as_str()]).await {
                    Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
                    _ => None,
                }
            } else {
                None
            };

            let mem = state.memory.lock().await;
            let episodes = mem.episodic.search(query_text);

            // Use vector search for facts when embedding is available
            let fact_results: Vec<serde_json::Value> = if let Some(ref qemb) = query_embedding {
                let vector_hits = mem.semantic.vector_search(qemb, 20);
                if !vector_hits.is_empty() {
                    vector_hits
                        .iter()
                        .map(|(f, score)| {
                            serde_json::json!({
                                "type": "fact",
                                "category": f.category,
                                "key": f.key,
                                "value": f.value,
                                "confidence": f.confidence,
                                "relevance": score,
                            })
                        })
                        .collect()
                } else {
                    mem.semantic
                        .search(query_text)
                        .iter()
                        .take(20)
                        .map(|f| {
                            serde_json::json!({
                                "type": "fact",
                                "category": f.category,
                                "key": f.key,
                                "value": f.value,
                                "confidence": f.confidence,
                            })
                        })
                        .collect()
                }
            } else {
                mem.semantic
                    .search(query_text)
                    .iter()
                    .take(20)
                    .map(|f| {
                        serde_json::json!({
                            "type": "fact",
                            "category": f.category,
                            "key": f.key,
                            "value": f.value,
                            "confidence": f.confidence,
                        })
                    })
                    .collect()
            };

            let ep_results: Vec<serde_json::Value> = episodes
                .iter()
                .take(10)
                .map(|e| {
                    serde_json::json!({
                        "type": "episode",
                        "summary": e.summary,
                        "outcome": e.outcome,
                        "tags": e.tags,
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            let mut results = ep_results;
            results.extend(fact_results);
            serde_json::json!({ "results": results, "query": query_text })
        }
        QueryKind::Config => {
            let channels = state.channels.lock().await;
            serde_json::json!({
                "agent": {
                    "model": &state.config.agent.model,
                    "fallback_model": &state.config.agent.fallback_model,
                    "fast_model": &state.config.agent.fast_model,
                    "max_tokens": state.config.agent.max_tokens,
                    "temperature": state.config.agent.temperature,
                    "max_iterations": state.config.agent.max_iterations,
                    "thinking_level": &state.config.agent.thinking_level,
                },
                "autonomy": {
                    "level": state.config.autonomy.level,
                    "daily_budget_usd": state.config.autonomy.daily_budget_usd,
                    "max_tool_calls_per_loop": state.config.autonomy.max_tool_calls_per_loop,
                    "approval_threshold": state.config.autonomy.approval_threshold,
                    "proactive": state.config.autonomy.proactive,
                    "tool_allowlist": &state.config.autonomy.tool_allowlist,
                    "tool_denylist": &state.config.autonomy.tool_denylist,
                },
                "memory": {
                    "db_path": state.config.memory.db_path.display().to_string(),
                    "max_episodes": state.config.memory.max_episodes,
                    "vector_search": state.config.memory.vector_search,
                    "embedding_dims": state.config.memory.embedding_dims,
                },
                "server": {
                    "listen": &state.config.server.listen,
                    "web_ui": state.config.server.web_ui,
                    "cors": state.config.server.cors,
                },
                "channels": channels.iter().map(|c| serde_json::json!({
                    "id": c.id(),
                    "type": c.channel_type(),
                })).collect::<Vec<_>>(),
                "plugins": {
                    "plugin_dir": state.config.plugins.plugin_dir.display().to_string(),
                    "registry_url": &state.config.plugins.registry_url,
                },
            })
        }
        QueryKind::AuditLog(limit) => {
            let mem = state.memory.lock().await;
            let entries: Vec<serde_json::Value> = mem
                .audit_log(limit)
                .into_iter()
                .map(|(timestamp, event_type, action, details)| {
                    serde_json::json!({
                        "timestamp": timestamp,
                        "event_type": event_type,
                        "action": action,
                        "details": details,
                    })
                })
                .collect();
            let count = entries.len();
            serde_json::json!({ "audit_log": entries, "count": count })
        }
        QueryKind::MeshPeers => {
            let mesh = state.mesh.lock().await;
            let peers: Vec<serde_json::Value> = mesh
                .peer_list()
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "peer_id": p.peer_id,
                        "hostname": p.hostname,
                        "capabilities": p.capabilities,
                        "os": p.os,
                    })
                })
                .collect();
            let count = peers.len();
            serde_json::json!({ "peers": peers, "count": count })
        }
        QueryKind::MeshStatus => {
            let mesh = state.mesh.lock().await;
            serde_json::json!({
                "enabled": state.config.mesh.enabled,
                "running": mesh.is_running(),
                "peer_id": mesh.peer_id(),
                "peer_count": mesh.peer_count(),
                "listen": &state.config.mesh.listen,
                "mdns": state.config.mesh.mdns,
                "capabilities": &state.config.mesh.capabilities,
                "p2p": true,
            })
        }
        QueryKind::SubTasks => {
            let tasks = state.pending_sub_tasks.lock().await;
            let list: Vec<serde_json::Value> = tasks
                .values()
                .map(|t| {
                    serde_json::json!({
                        "task_id": t.task_id.to_string(),
                        "role": t.role,
                        "task_description": t.task_description,
                        "status": t.status,
                        "result": t.result,
                        "error": t.error,
                        "depends_on": t.depends_on.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                        "elapsed_secs": t.created_at.elapsed().as_secs(),
                    })
                })
                .collect();
            let count = list.len();
            let running = list.iter().filter(|t| t["status"] == "running").count();
            let completed = list.iter().filter(|t| t["status"] == "completed").count();
            let failed = list.iter().filter(|t| t["status"] == "failed").count();
            serde_json::json!({
                "sub_tasks": list,
                "count": count,
                "running": running,
                "completed": completed,
                "failed": failed,
            })
        }
        QueryKind::ScheduledTasks => {
            if let Some(ref scheduler) = state.scheduler {
                let tasks = scheduler.list_all().await;
                let list: Vec<serde_json::Value> = tasks
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id.to_string(),
                            "label": t.label,
                            "description": t.description,
                            "kind": t.kind,
                            "active": t.active,
                            "fire_count": t.fire_count,
                            "last_fired": t.last_fired.map(|d| d.to_rfc3339()),
                            "created_at": t.created_at.to_rfc3339(),
                        })
                    })
                    .collect();
                let active = list.iter().filter(|t| t["active"] == true).count();
                serde_json::json!({
                    "scheduled_tasks": list,
                    "count": list.len(),
                    "active": active,
                })
            } else {
                serde_json::json!({
                    "scheduled_tasks": [],
                    "count": 0,
                    "active": 0,
                    "scheduler_enabled": false,
                })
            }
        }
    };
    Ok(result)
}
