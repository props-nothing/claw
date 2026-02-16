use std::sync::Arc;

use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

use claw_core::{ToolCall, ToolResult};
use claw_device::DeviceTools;
use claw_llm::LlmRequest;
use claw_mesh::MeshMessage;

use crate::agent::{MeshTaskResult, SharedAgentState};
use crate::learning::extract_search_keywords;
use crate::sub_agent::{
    exec_cron_cancel, exec_cron_list, exec_cron_schedule, exec_sub_agent_spawn,
    exec_sub_agent_status, exec_sub_agent_wait,
};

pub(crate) async fn execute_tool_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    debug!(tool = %call.tool_name, "executing tool");

    // Memory and goal tools need locks on shared state
    match call.tool_name.as_str() {
        "memory_search" => return exec_memory_search_shared(state, call).await,
        "memory_store" => return exec_memory_store_shared(state, call).await,
        "memory_delete" => return exec_memory_delete_shared(state, call).await,
        "memory_list" => return exec_memory_list_shared(state, call).await,
        "goal_create" => return exec_goal_create_shared(state, call).await,
        "goal_list" => return exec_goal_list_shared(state, call).await,
        "goal_complete_step" => return exec_goal_complete_step_shared(state, call).await,
        "goal_update_status" => return exec_goal_update_status_shared(state, call).await,
        "llm_generate" => return exec_llm_generate_shared(state, call).await,
        "web_search" => return exec_web_search_shared(state, call).await,
        "mesh_peers" => return exec_mesh_peers_shared(state, call).await,
        "mesh_delegate" => return exec_mesh_delegate_shared(state, call).await,
        "mesh_status" => return exec_mesh_status_shared(state, call).await,
        "channel_send_file" => return exec_channel_send_file(state, call).await,
        "sub_agent_spawn" => return exec_sub_agent_spawn(state, call).await,
        "sub_agent_wait" => return exec_sub_agent_wait(state, call).await,
        "sub_agent_status" => return exec_sub_agent_status(state, call).await,
        "cron_schedule" => return exec_cron_schedule(state, call).await,
        "cron_list" => return exec_cron_list(state, call).await,
        "cron_cancel" => return exec_cron_cancel(state, call).await,
        _ => {}
    }

    // Builtin tools (shell, file ops) — no lock needed
    if state.tools.has_tool(&call.tool_name) {
        match state.tools.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Error: {e}"),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    // Device tools — browser_*, android_*, ios_*
    if DeviceTools::has_tool(&call.tool_name) {
        match state.device_tools.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Device error: {e}"),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    // Plugin tools — "pluginname_toolname" (underscore-separated, plugin prefix matched)
    if state.plugins.is_plugin_tool(&call.tool_name) {
        match state.plugins.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Plugin error: {e}"),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Tool not found: {}", call.tool_name),
        is_error: true,
        data: None,
    }
}

async fn exec_llm_generate_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let prompt = match call.arguments["prompt"].as_str() {
        Some(p) => p,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'prompt' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let max_tokens = call.arguments["max_tokens"].as_u64().unwrap_or(2048) as u32;

    let request = LlmRequest {
        model: state.config.agent.model.clone(),
        messages: vec![claw_core::Message {
            id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::nil(),
            role: claw_core::Role::User,
            content: vec![claw_core::MessageContent::Text {
                text: prompt.to_string(),
            }],
            timestamp: chrono::Utc::now(),
            tool_calls: vec![],
            metadata: serde_json::Map::new(),
        }],
        tools: Arc::new(vec![]),
        system: Some("You are a helpful assistant. Respond concisely and directly.".into()),
        max_tokens,
        temperature: state.config.agent.temperature,
        thinking_level: None,
        stream: false,
    };

    match state
        .llm
        .complete(&request, state.config.agent.fallback_model.as_deref())
        .await
    {
        Ok(response) => {
            let text = response
                .message
                .content
                .iter()
                .filter_map(|c| match c {
                    claw_core::MessageContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            ToolResult {
                tool_call_id: call.id.clone(),
                content: text,
                is_error: false,
                data: None,
            }
        }
        Err(e) => ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("LLM generation failed: {e}"),
            is_error: true,
            data: None,
        },
    }
}

/// Execute a web search using the Brave Search API.
async fn exec_web_search_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let query = match call.arguments["query"].as_str() {
        Some(q) => q,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'query' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let count = call.arguments["count"].as_u64().unwrap_or(5).min(20) as u32;

    let api_key = match &state.config.services.brave_api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Web search is not configured. To enable it:\n\
                    1. Get a free API key at https://api.search.brave.com/\n\
                    2. Add to your config: claw set services.brave_api_key YOUR_KEY\n\
                    3. Or run: claw setup"
                    .into(),
                is_error: true,
                data: None,
            };
        }
    };

    info!(query = query, count = count, "executing web search");

    let resp = match state.http_client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &api_key)
        .query(&[("q", query), ("count", &count.to_string())])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Web search request failed: {e}"),
                is_error: true,
                data: None,
            };
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Brave Search API error ({status}): {body}"),
            is_error: true,
            data: None,
        };
    }

    let data: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to parse search results: {e}"),
                is_error: true,
                data: None,
            };
        }
    };

    // Extract web results
    let mut results = Vec::new();
    if let Some(web_results) = data["web"]
        .as_object()
        .and_then(|w| w["results"].as_array())
    {
        for (i, result) in web_results.iter().enumerate() {
            let title = result["title"].as_str().unwrap_or("Untitled");
            let url = result["url"].as_str().unwrap_or("");
            let description = result["description"].as_str().unwrap_or("");
            results.push(format!(
                "{}. {}\n   {}\n   {}",
                i + 1,
                title,
                url,
                description
            ));
        }
    }

    if results.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("No results found for: {query}"),
            is_error: false,
            data: None,
        };
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Search results for '{}':\n\n{}",
            query,
            results.join("\n\n")
        ),
        is_error: false,
        data: None,
    }
}

// ─── Mesh tool implementations ──────────────────────────────────────────────

/// List connected mesh peers and their capabilities.
async fn exec_mesh_peers_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let capability_filter = call.arguments.get("capability").and_then(|v| v.as_str());

    let mesh = state.mesh.lock().await;
    if !mesh.is_running() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content:
                "Mesh networking is not running. Enable it in your config: [mesh] enabled = true"
                    .into(),
            is_error: true,
            data: None,
        };
    }

    let peers: Vec<_> = mesh
        .peer_list()
        .into_iter()
        .filter(|p| {
            if let Some(cap) = capability_filter {
                p.capabilities.iter().any(|c| c == cap)
            } else {
                true
            }
        })
        .collect();

    if peers.is_empty() {
        let msg = if let Some(cap) = capability_filter {
            format!(
                "No peers found with capability '{}'. {} total peers connected.",
                cap,
                mesh.peer_count()
            )
        } else {
            "No peers connected to the mesh.".to_string()
        };
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: msg,
            is_error: false,
            data: None,
        };
    }

    let mut lines = vec![format!("Connected peers ({}):", peers.len())];
    for p in &peers {
        lines.push(format!(
            "  • {} ({}) — capabilities: [{}], os: {}",
            p.hostname,
            &p.peer_id[..8.min(p.peer_id.len())],
            p.capabilities.join(", "),
            p.os,
        ));
    }

    let peer_data: Vec<serde_json::Value> = peers
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

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: Some(serde_json::json!({ "peers": peer_data })),
    }
}

/// Delegate a task to a mesh peer and await the result.
async fn exec_mesh_delegate_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_desc = match call.arguments.get("task").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'task' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let explicit_peer = call.arguments.get("peer_id").and_then(|v| v.as_str());
    let capability = call.arguments.get("capability").and_then(|v| v.as_str());
    let priority = call
        .arguments
        .get("priority")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u8;
    let timeout_secs = call
        .arguments
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120);

    // Resolve target peer
    let (target_peer_id, target_hostname) = {
        let mesh = state.mesh.lock().await;
        if !mesh.is_running() {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Mesh networking is not running.".into(),
                is_error: true,
                data: None,
            };
        }

        if let Some(pid) = explicit_peer {
            // Verify the peer exists
            match mesh.peers().get(pid) {
                Some(p) => (pid.to_string(), p.hostname.clone()),
                None => {
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "Peer '{pid}' not found in mesh. Use mesh_peers to see available peers."
                        ),
                        is_error: true,
                        data: None,
                    };
                }
            }
        } else if let Some(cap) = capability {
            match mesh.find_best_peer_for_capability(cap) {
                Some(p) => (p.peer_id.clone(), p.hostname.clone()),
                None => {
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "No peer with capability '{}' found. Available peers: {}",
                            cap,
                            mesh.peer_list()
                                .iter()
                                .map(|p| format!("{} [{}]", p.hostname, p.capabilities.join(",")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        is_error: true,
                        data: None,
                    };
                }
            }
        } else {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content:
                    "Error: must provide either 'peer_id' or 'capability' to select a target peer."
                        .into(),
                is_error: true,
                data: None,
            };
        }
    };

    // Build the task assignment
    let our_peer_id = {
        let mesh = state.mesh.lock().await;
        mesh.peer_id().to_string()
    };

    let mut task = claw_mesh::TaskAssignment::new(&our_peer_id, &target_peer_id, &task_desc)
        .with_priority(priority);

    if let Some(cap) = capability {
        task = task.with_capability(cap);
    }

    let task_id = task.task_id;

    info!(
        task_id = %task_id,
        target_peer = %target_peer_id,
        target_host = %target_hostname,
        task = %task_desc,
        "delegating task to mesh peer"
    );

    // Register a oneshot channel to await the result
    let (result_tx, result_rx) = oneshot::channel::<MeshTaskResult>();
    {
        state
            .pending_mesh_tasks
            .lock()
            .await
            .insert(task_id, result_tx);
    }

    // Send the task via mesh
    let msg = MeshMessage::TaskAssign(task);
    {
        let mesh = state.mesh.lock().await;
        if let Err(e) = mesh.send_to(&target_peer_id, &msg).await {
            // Clean up pending task
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to send task to peer: {e}"),
                is_error: true,
                data: None,
            };
        }
    }

    // Await the result with timeout
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), result_rx).await {
        Ok(Ok(result)) => {
            info!(
                task_id = %task_id,
                peer = %result.peer_id,
                success = result.success,
                "received delegated task result"
            );
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Task delegated to {} ({}) — {}\n\nResult:\n{}",
                    target_hostname,
                    &target_peer_id[..8.min(target_peer_id.len())],
                    if result.success { "SUCCESS" } else { "FAILED" },
                    result.result,
                ),
                is_error: !result.success,
                data: Some(serde_json::json!({
                    "task_id": task_id.to_string(),
                    "peer_id": result.peer_id,
                    "success": result.success,
                })),
            }
        }
        Ok(Err(_)) => {
            // Channel dropped — peer disconnected or runtime shutting down
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Task {task_id} was cancelled — peer may have disconnected."),
                is_error: true,
                data: None,
            }
        }
        Err(_) => {
            // Timeout
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Task {} timed out after {}s waiting for response from {} ({}).",
                    task_id,
                    timeout_secs,
                    target_hostname,
                    &target_peer_id[..8.min(target_peer_id.len())]
                ),
                is_error: true,
                data: None,
            }
        }
    }
}

/// Get the status of the mesh network.
async fn exec_mesh_status_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let mesh = state.mesh.lock().await;
    let running = mesh.is_running();
    let peer_id = mesh.peer_id().to_string();
    let peer_count = mesh.peer_count();
    drop(mesh);

    let status = if running {
        format!(
            "Mesh network: RUNNING\n\
             Our peer ID: {}\n\
             Connected peers: {}\n\
             Listen address: {}\n\
             mDNS discovery: {}\n\
             Our capabilities: [{}]",
            &peer_id[..12.min(peer_id.len())],
            peer_count,
            state.config.mesh.listen,
            if state.config.mesh.mdns {
                "enabled"
            } else {
                "disabled"
            },
            state.config.mesh.capabilities.join(", "),
        )
    } else {
        "Mesh network: NOT RUNNING\nEnable it in config: [mesh] enabled = true".to_string()
    };

    ToolResult {
        tool_call_id: call.id.clone(),
        content: status,
        is_error: false,
        data: Some(serde_json::json!({
            "running": running,
            "peer_id": peer_id,
            "peer_count": peer_count,
            "capabilities": state.config.mesh.capabilities,
        })),
    }
}

async fn exec_memory_search_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let query = call.arguments["query"].as_str().unwrap_or("");
    let mem_type = call.arguments["type"].as_str().unwrap_or("all");

    // Generate query embedding for vector search
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed(&[query]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            _ => None,
        }
    } else {
        None
    };

    let mem = state.memory.read().await;
    let mut results = Vec::new();

    if mem_type == "episodic" || mem_type == "all" {
        let episodes = mem.episodic.search(query);
        for ep in episodes.iter().take(10) {
            results.push(format!(
                "[Episode {}] {}{}",
                ep.created_at.format("%Y-%m-%d"),
                ep.summary,
                ep.outcome
                    .as_ref()
                    .map(|o| format!(" → {o}"))
                    .unwrap_or_default()
            ));
        }
    }

    if mem_type == "semantic" || mem_type == "all" {
        // Combine vector + keyword search, dedup by category:key
        let mut seen = std::collections::HashSet::new();

        // Vector search first (highest quality)
        if let Some(ref qemb) = query_embedding {
            for (fact, score) in mem.semantic.vector_search(qemb, 15) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen.insert(fk) {
                    results.push(format!(
                        "[Fact: {}/{}] {} (relevance: {:.0}%)",
                        fact.category,
                        fact.key,
                        fact.value,
                        score * 100.0
                    ));
                }
            }
        }

        // Word-level keyword search (catches things without embeddings)
        for fact in mem.semantic.search(query).iter().take(15) {
            let fk = format!("{}:{}", fact.category, fact.key);
            if seen.insert(fk) {
                results.push(format!(
                    "[Fact: {}/{}] {} (confidence: {:.0}%)",
                    fact.category,
                    fact.key,
                    fact.value,
                    fact.confidence * 100.0
                ));
            }
        }

        // Also search with extracted keywords for broader matching
        let keywords = extract_search_keywords(query);
        if keywords != query.to_lowercase() {
            for fact in mem.semantic.search(&keywords).iter().take(5) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen.insert(fk) {
                    results.push(format!(
                        "[Fact: {}/{}] {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }
    }

    let content = if results.is_empty() {
        format!(
            "No relevant memories found for query: \"{query}\". Try memory_list to see all stored facts."
        )
    } else {
        results.join("\n")
    };

    ToolResult {
        tool_call_id: call.id.clone(),
        content,
        is_error: false,
        data: None,
    }
}

async fn exec_memory_store_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let category = call.arguments["category"].as_str().unwrap_or("general");
    let key = call.arguments["key"].as_str().unwrap_or("unknown");
    let value = call.arguments["value"].as_str().unwrap_or("");

    // Generate embedding if an embedder is configured
    let embedding = if let Some(ref embedder) = state.embedder {
        let text_for_embedding = format!("{category} {key} {value}");
        match embedder.embed(&[&text_for_embedding]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "failed to generate embedding for fact, storing without vector");
                None
            }
        }
    } else {
        None
    };

    let mut mem = state.memory.write().await;

    let fact = claw_memory::semantic::Fact {
        id: Uuid::new_v4(),
        category: category.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        confidence: 1.0,
        source: Some("agent".to_string()),
        embedding: embedding.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    mem.semantic.upsert(fact);

    if let Err(e) = mem.persist_fact_with_embedding(category, key, value, embedding.as_deref()) {
        warn!(error = %e, "failed to persist fact to SQLite");
    }
    drop(mem); // Release memory lock before mesh operations

    // Broadcast fact to mesh peers for sync
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() && mesh.peer_count() > 0 {
            let sync_msg = MeshMessage::SyncDelta {
                peer_id: mesh.peer_id().to_string(),
                delta_type: "fact".to_string(),
                data: serde_json::json!({
                    "category": category,
                    "key": key,
                    "value": value,
                    "confidence": 1.0,
                }),
            };
            if let Err(e) = mesh.broadcast(&sync_msg).await {
                debug!(error = %e, "failed to broadcast fact to mesh peers");
            } else {
                debug!(
                    category = category,
                    key = key,
                    "broadcast fact to mesh peers"
                );
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Stored fact: {category}/{key} = {value}"),
        is_error: false,
        data: None,
    }
}

async fn exec_memory_delete_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let category = call.arguments["category"].as_str().unwrap_or("");
    let key = call.arguments.get("key").and_then(|v| v.as_str());

    if category.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: 'category' is required".to_string(),
            is_error: true,
            data: None,
        };
    }

    let mut mem = state.memory.write().await;

    let result_msg = if let Some(key) = key {
        // Delete a specific fact
        let removed_mem = mem.semantic.remove(category, key);
        let removed_db = mem.delete_fact(category, key).unwrap_or(false);
        if removed_mem || removed_db {
            format!("Deleted fact: {category}/{key}")
        } else {
            format!("Fact not found: {category}/{key}")
        }
    } else {
        // Delete entire category
        let count_mem = mem.semantic.remove_category(category);
        let count_db = mem.delete_facts_by_category(category).unwrap_or(0);
        let count = count_mem.max(count_db);
        if count > 0 {
            format!("Deleted {count} fact(s) from category '{category}'")
        } else {
            format!("Category '{category}' not found or already empty")
        }
    };

    drop(mem);

    ToolResult {
        tool_call_id: call.id.clone(),
        content: result_msg,
        is_error: false,
        data: None,
    }
}

async fn exec_memory_list_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let filter_category = call.arguments.get("category").and_then(|v| v.as_str());

    let mem = state.memory.read().await;
    let mut lines = Vec::new();

    if let Some(cat) = filter_category {
        // List facts in a specific category
        let facts = mem.semantic.category(cat);
        if facts.is_empty() {
            lines.push(format!("Category '{cat}': (empty)"));
        } else {
            lines.push(format!("Category '{}' ({} facts):", cat, facts.len()));
            for fact in facts {
                lines.push(format!(
                    "  - {}: {} (confidence: {:.0}%, updated: {})",
                    fact.key,
                    fact.value,
                    fact.confidence * 100.0,
                    fact.updated_at.format("%Y-%m-%d %H:%M")
                ));
            }
        }
    } else {
        // List all categories with their facts
        let mut categories: Vec<&str> = mem.semantic.categories();
        categories.sort();
        if categories.is_empty() {
            lines.push("Memory is empty — no facts stored.".to_string());
        } else {
            let total = mem.semantic.count();
            lines.push(format!(
                "Total: {} facts across {} categories\n",
                total,
                categories.len()
            ));
            for cat in categories {
                let facts = mem.semantic.category(cat);
                lines.push(format!("📁 {} ({}):", cat, facts.len()));
                for fact in facts.iter().take(20) {
                    lines.push(format!(
                        "  - {}: {}",
                        fact.key,
                        if fact.value.len() > 120 {
                            format!("{}…", &fact.value[..120])
                        } else {
                            fact.value.clone()
                        }
                    ));
                }
                if facts.len() > 20 {
                    lines.push(format!("  ... and {} more", facts.len() - 20));
                }
            }
        }
    }

    drop(mem);

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_create_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let priority = call.arguments["priority"].as_u64().unwrap_or(5) as u8;
    let steps: Vec<String> = call.arguments["steps"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let mut planner = state.planner.lock().await;
    let goal = planner.create_goal(description.clone(), priority);
    let goal_id = goal.id;
    if !steps.is_empty() {
        planner.set_plan(goal_id, steps.clone());
    }

    // Persist goal to SQLite
    {
        let mem = state.memory.read().await;
        if let Err(e) = mem.persist_goal(&goal_id, &description, "active", priority, 0.0, None) {
            warn!(error = %e, "failed to persist goal to SQLite");
        }
        // Persist steps
        if let Some(goal) = planner.get(goal_id) {
            for step in &goal.steps {
                if let Err(e) = mem.persist_goal_step(
                    &step.id,
                    &goal_id,
                    &step.description,
                    &format!("{:?}", step.status).to_lowercase(),
                    None,
                ) {
                    warn!(error = %e, "failed to persist goal step to SQLite");
                }
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Created goal '{}' (id: {}, priority: {}, {} steps)",
            description,
            goal_id,
            priority,
            steps.len()
        ),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_list_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let planner = state.planner.lock().await;
    let goals = planner.all();
    if goals.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No goals.".to_string(),
            is_error: false,
            data: None,
        };
    }

    let mut lines = Vec::new();
    for goal in goals {
        let status_tag = match goal.status {
            claw_autonomy::planner::GoalStatus::Completed => " ✅ COMPLETED",
            claw_autonomy::planner::GoalStatus::Cancelled => " ❌ CANCELLED",
            _ => "",
        };
        lines.push(format!(
            "• [{}] {} (priority: {}, progress: {:.0}%{})",
            goal.id,
            goal.description,
            goal.priority,
            goal.progress * 100.0,
            status_tag
        ));
        for step in &goal.steps {
            let icon = match step.status {
                claw_autonomy::planner::StepStatus::Completed => "✅",
                claw_autonomy::planner::StepStatus::InProgress => "🔄",
                claw_autonomy::planner::StepStatus::Failed => "❌",
                _ => "⬜",
            };
            lines.push(format!(
                "    {} [step:{}] {}",
                icon, step.id, step.description
            ));
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_complete_step_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let goal_id_str = call.arguments["goal_id"].as_str().unwrap_or("");
    let step_id_str = call.arguments["step_id"].as_str().unwrap_or("");
    let result = call.arguments["result"].as_str().unwrap_or("").to_string();

    let goal_id = match goal_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid goal_id: {goal_id_str}"),
                is_error: true,
                data: None,
            };
        }
    };
    let step_id = match step_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid step_id: {step_id_str}"),
                is_error: true,
                data: None,
            };
        }
    };

    let mut planner = state.planner.lock().await;
    planner.complete_step(goal_id, step_id, result.clone());

    // Get updated progress
    let (progress, status) = planner
        .get(goal_id)
        .map(|g| (g.progress, format!("{:?}", g.status)))
        .unwrap_or((0.0, "unknown".into()));

    // Persist updated goal to SQLite
    {
        let goal_desc = planner
            .get(goal_id)
            .map(|g| g.description.clone())
            .unwrap_or_default();
        let goal_priority = planner.get(goal_id).map(|g| g.priority).unwrap_or(5);
        let step_desc = planner
            .get(goal_id)
            .and_then(|g| g.steps.iter().find(|s| s.id == step_id))
            .map(|s| s.description.clone())
            .unwrap_or_default();
        let mem = state.memory.read().await;
        let _ = mem.persist_goal(
            &goal_id,
            &goal_desc,
            &status.to_lowercase(),
            goal_priority,
            progress,
            None,
        );
        let _ = mem.persist_goal_step(&step_id, &goal_id, &step_desc, "completed", Some(&result));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Step completed. Goal progress: {:.0}%, status: {}",
            progress * 100.0,
            status
        ),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_update_status_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let goal_id_str = call.arguments["goal_id"].as_str().unwrap_or("");
    let status_str = call.arguments["status"].as_str().unwrap_or("active");
    let reason = call.arguments["reason"].as_str().unwrap_or("").to_string();

    let goal_id = match goal_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid goal_id: {goal_id_str}"),
                is_error: true,
                data: None,
            };
        }
    };

    let mut planner = state.planner.lock().await;

    // Find the goal and update its status
    let updated = if let Some(goal) = planner.all_mut().iter_mut().find(|g| g.id == goal_id) {
        let new_status = match status_str {
            "completed" => claw_autonomy::planner::GoalStatus::Completed,
            "failed" => claw_autonomy::planner::GoalStatus::Failed,
            "paused" => claw_autonomy::planner::GoalStatus::Paused,
            "cancelled" => claw_autonomy::planner::GoalStatus::Cancelled,
            "active" => claw_autonomy::planner::GoalStatus::Active,
            _ => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Invalid status: {status_str}. Use: active, completed, failed, paused, cancelled"
                    ),
                    is_error: true,
                    data: None,
                };
            }
        };
        goal.status = new_status;
        if !reason.is_empty() {
            goal.retrospective = Some(reason.clone());
        }
        goal.updated_at = chrono::Utc::now();
        true
    } else {
        false
    };

    if !updated {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Goal not found: {goal_id_str}"),
            is_error: true,
            data: None,
        };
    }

    // Persist updated goal to SQLite — read current values so we don't clobber description/priority/progress
    {
        let goal_desc = planner
            .get(goal_id)
            .map(|g| g.description.clone())
            .unwrap_or_default();
        let goal_priority = planner.get(goal_id).map(|g| g.priority).unwrap_or(5);
        let goal_progress = planner.get(goal_id).map(|g| g.progress).unwrap_or(0.0);
        let mem = state.memory.read().await;
        let _ = mem.persist_goal(
            &goal_id,
            &goal_desc,
            status_str,
            goal_priority,
            goal_progress,
            None,
        );
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Goal {} status updated to '{}'{}",
            goal_id,
            status_str,
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        ),
        is_error: false,
        data: None,
    }
}

/// Execute channel_send_file — send a file through the active chat channel.
pub(crate) async fn exec_channel_send_file(
    state: &SharedAgentState,
    call: &ToolCall,
) -> ToolResult {
    use claw_channels::adapter::{Attachment, OutgoingMessage};

    let file_path_raw = match call.arguments["file_path"].as_str() {
        Some(p) => p,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'file_path' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    // Expand ~ to home directory
    let file_path_str = if file_path_raw == "~" || file_path_raw.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            format!("{}{}", home.display(), &file_path_raw[1..])
        } else {
            file_path_raw.to_string()
        }
    } else {
        file_path_raw.to_string()
    };

    let file_path = std::path::Path::new(&file_path_str);

    // Verify file exists
    if !file_path.exists() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Error: file not found: {file_path_str}"),
            is_error: true,
            data: None,
        };
    }

    // Read the reply context to know which channel/target to send to
    let (channel_id, target) = {
        let ctx = state.reply_context.lock().await;
        match ctx.as_ref() {
            Some((cid, tgt)) => (cid.clone(), tgt.clone()),
            None => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: "Error: no active channel context — channel_send_file can only be used when responding to a channel message".into(),
                    is_error: true,
                    data: None,
                };
            }
        }
    };

    // Determine MIME type from extension
    let filename = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let lower = filename.to_lowercase();
    let media_type = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".m4a") {
        "audio/mp4"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".aac") {
        "audio/aac"
    } else if lower.ends_with(".flac") {
        "audio/flac"
    } else if lower.ends_with(".aiff") {
        "audio/aiff"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else if lower.ends_with(".avi") {
        "video/x-msvideo"
    } else if lower.ends_with(".mkv") {
        "video/x-matroska"
    } else if lower.ends_with(".webm") {
        "video/webm"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".txt") || lower.ends_with(".log") || lower.ends_with(".md") {
        "text/plain"
    } else {
        "application/octet-stream"
    };

    let caption = call.arguments["caption"].as_str().unwrap_or("").to_string();

    // Send through the channel with the file as an attachment
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            let msg = OutgoingMessage {
                channel: channel_id.clone(),
                target: target.clone(),
                text: caption.clone(),
                attachments: vec![Attachment {
                    filename: filename.clone(),
                    media_type: media_type.to_string(),
                    data: file_path_str.clone(), // Pass the file path — adapters read it
                }],
                reply_to: None,
            };

            match channel.send(msg).await {
                Ok(()) => {
                    info!(file = %file_path_str, channel = %channel_id, "channel_send_file: file sent successfully");
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "File sent successfully: {filename} ({media_type}, {channel_id})"
                        ),
                        is_error: false,
                        data: None,
                    };
                }
                Err(e) => {
                    warn!(error = %e, file = %file_path_str, "channel_send_file: failed to send");
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!("Error sending file: {e}"),
                        is_error: true,
                        data: None,
                    };
                }
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Error: channel '{channel_id}' not found"),
        is_error: true,
        data: None,
    }
}

/// Check if a tool can safely be executed in parallel with other tools.
/// Tools that don't mutate shared state or that operate on independent resources are parallel-safe.
pub(crate) fn is_parallel_safe(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "http_fetch"
            | "web_search"
            | "file_read"
            | "file_list"
            | "file_find"
            | "file_grep"
            | "memory_search"
            | "memory_list"
            | "mesh_peers"
            | "mesh_delegate"
            | "mesh_status"
            | "goal_list"
            | "sub_agent_spawn"
            | "sub_agent_status"
            | "process_list"
            | "process_output"
            | "terminal_view"
    )
}
