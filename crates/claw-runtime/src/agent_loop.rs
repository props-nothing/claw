use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use claw_autonomy::{ApprovalResponse, AutonomyLevel, guardrail::GuardrailVerdict};
use claw_channels::adapter::IncomingMessage;
use claw_core::{Message, Role, Tool, ToolResult};
use claw_device::DeviceTools;
use claw_llm::{LlmRequest, StopReason};
use claw_mesh::MeshMessage;

use crate::agent::{
    ApiResponse, MeshTaskResult, SharedAgentState, StreamEvent, build_default_system_prompt,
};
use crate::channel_helpers::{
    describe_tool_call, edit_channel_message, extract_result_summary, send_approval_prompt_shared,
    send_channel_message_returning_id, send_response_shared, send_typing_to_channel,
    tool_progress_emoji,
};
use crate::learning::{
    build_episode_summary, extract_episode_tags, extract_search_keywords, maybe_extract_lessons,
};
use crate::tool_dispatch::{execute_tool_shared, is_parallel_safe};

pub(crate) async fn process_mesh_message(state: SharedAgentState, message: MeshMessage) {
    let our_peer_id = {
        let mesh = state.mesh.lock().await;
        mesh.peer_id().to_string()
    };

    // Only process messages addressed to us
    if !message.is_for_peer(&our_peer_id) {
        return;
    }

    // Let the mesh node update its peer table
    let handled = {
        let mut mesh = state.mesh.lock().await;
        mesh.handle_message(&message)
    };

    if handled {
        return; // Peer bookkeeping only — no further action needed
    }

    // Handle messages that require runtime processing
    match message {
        MeshMessage::TaskAssign(task) => {
            info!(
                task_id = %task.task_id,
                from = %task.from_peer,
                desc = %task.description,
                "received task assignment from mesh peer"
            );

            // Execute the task by processing it as a chat message
            let result_text =
                match process_api_message(state.clone(), task.description.clone(), None).await {
                    resp if resp.error.is_none() => resp.text,
                    resp => format!("Error: {}", resp.error.unwrap_or_default()),
                };

            // Send the result back to the originator
            let result_msg = MeshMessage::TaskResult {
                task_id: task.task_id,
                peer_id: our_peer_id.clone(),
                success: true,
                result: result_text,
            };
            let mesh = state.mesh.lock().await;
            if let Err(e) = mesh.send_to(&task.from_peer, &result_msg).await {
                warn!(error = %e, from = %task.from_peer, "failed to send task result");
            }
        }
        MeshMessage::TaskResult {
            task_id,
            peer_id,
            success,
            result,
        } => {
            info!(
                task_id = %task_id,
                from = %peer_id,
                success = success,
                "received task result from mesh peer"
            );

            // Resolve the pending mesh task if someone is waiting for it
            let resolved = {
                let mut pending = state.pending_mesh_tasks.lock().await;
                if let Some(tx) = pending.remove(&task_id) {
                    let _ = tx.send(MeshTaskResult {
                        task_id,
                        peer_id: peer_id.clone(),
                        success,
                        result: result.clone(),
                    });
                    true
                } else {
                    false
                }
            };

            if !resolved {
                // No one waiting — check if it's for a delegated goal step
                let mut planner = state.planner.lock().await;
                if success {
                    planner.complete_delegated_task(task_id, result.clone());
                } else {
                    planner.fail_delegated_task(task_id, result.clone());
                }
            }
        }
        MeshMessage::DirectMessage {
            from_peer, content, ..
        } => {
            info!(
                from = %from_peer,
                content = %content,
                "received direct message from mesh peer"
            );
        }
        MeshMessage::SyncDelta {
            peer_id,
            delta_type,
            data,
        } => {
            debug!(
                from = %peer_id,
                delta_type = %delta_type,
                "received sync delta from mesh peer"
            );

            match delta_type.as_str() {
                "fact" => {
                    // Apply incoming fact to our local memory
                    if let (Some(category), Some(key), Some(value)) = (
                        data.get("category").and_then(|v| v.as_str()),
                        data.get("key").and_then(|v| v.as_str()),
                        data.get("value").and_then(|v| v.as_str()),
                    ) {
                        let confidence = data
                            .get("confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.8);
                        let source = format!("mesh:{peer_id}");
                        let mut mem = state.memory.write().await;
                        // Upsert into in-memory semantic store
                        mem.semantic.upsert(claw_memory::semantic::Fact {
                            id: uuid::Uuid::new_v4(),
                            category: category.to_string(),
                            key: key.to_string(),
                            value: value.to_string(),
                            confidence,
                            source: Some(source.clone()),
                            embedding: None,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                        });
                        // Persist to SQLite
                        let _ = mem.persist_fact(category, key, value);
                        info!(
                            category = category,
                            key = key,
                            from = %peer_id,
                            "synced fact from mesh peer"
                        );
                    } else {
                        warn!(from = %peer_id, "received malformed fact sync delta");
                    }
                }
                "episode" => {
                    // Apply incoming episode summary
                    if let Some(summary) = data.get("summary").and_then(|v| v.as_str()) {
                        let outcome = data
                            .get("outcome")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let tags: Vec<String> = data
                            .get("tags")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut mem = state.memory.write().await;
                        let episode = claw_memory::episodic::Episode {
                            id: uuid::Uuid::new_v4(),
                            session_id: uuid::Uuid::new_v4(),
                            summary: summary.to_string(),
                            outcome,
                            tags,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                        };
                        mem.episodic.record(episode);
                        info!(
                            summary = summary,
                            from = %peer_id,
                            "synced episode from mesh peer"
                        );
                    }
                }
                other => {
                    debug!(delta_type = other, "unknown sync delta type — ignoring");
                }
            }
        }
        _ => {}
    }
}

/// Process a non-streaming API chat message — spawned as a concurrent task.
pub(crate) async fn process_api_message(
    state: SharedAgentState,
    text: String,
    session_id_hint: Option<String>,
) -> ApiResponse {
    let session_id = if let Some(ref hint) = session_id_hint {
        if let Ok(id) = hint.parse::<Uuid>() {
            state.sessions.get_or_insert(id, "api", "api_user").await
        } else {
            state.sessions.find_or_create("api", hint).await
        }
    } else {
        state.sessions.create_for_channel("api", "api_user").await
    };

    let incoming = IncomingMessage {
        id: Uuid::new_v4().to_string(),
        channel: "api".to_string(),
        sender: "api_user".to_string(),
        sender_name: Some("API User".to_string()),
        group: None,
        text: Some(text),
        attachments: vec![],
        is_mention: false,
        is_reply_to_bot: false,
        metadata: serde_json::Value::Null,
    };

    match process_message_shared(&state, "api", incoming, Some(session_id)).await {
        Ok(response_text) => ApiResponse {
            text: response_text,
            session_id: session_id.to_string(),
            error: None,
        },
        Err(e) => ApiResponse {
            text: String::new(),
            session_id: session_id.to_string(),
            error: Some(e.to_string()),
        },
    }
}

/// Process a channel message — spawned as a concurrent task.
/// Uses the streaming path so we can send real-time progress updates
/// (typing indicators, tool-call notifications) to the channel while
/// the agent works through multi-step tasks.
pub(crate) async fn process_channel_message(
    state: SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
) -> claw_core::Result<()> {
    let target = incoming
        .group
        .as_deref()
        .unwrap_or(&incoming.sender)
        .to_string();
    let channel_id_owned = channel_id.to_string();

    // Spawn periodic typing indicator so the user sees activity
    // (Telegram typing indicators expire after ~5 s)
    let state_typing = state.clone();
    let cid_typing = channel_id_owned.clone();
    let target_typing = target.clone();
    let typing_handle = tokio::spawn(async move {
        loop {
            send_typing_to_channel(&state_typing, &cid_typing, &target_typing).await;
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
    });

    // Create streaming channel
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(128);

    // Spawn the streaming processor
    let state_stream = state.clone();
    let cid_stream = channel_id_owned.clone();
    let stream_handle = tokio::spawn(async move {
        let result =
            process_message_streaming_shared(&state_stream, &cid_stream, incoming, &tx, None).await;
        match &result {
            Ok(()) => {
                let _ = tx.send(StreamEvent::Done).await;
            }
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
            }
        }
        result
    });

    // Consume stream events and forward progress as a single live-edited message
    let mut final_text = String::new();
    let mut progress_lines: Vec<String> = Vec::new();
    let mut progress_msg_id: Option<String> = None;
    let mut current_tool_ids: HashMap<String, usize> = HashMap::new(); // tool_call_id → index in progress_lines
    let mut last_edit_time = std::time::Instant::now() - std::time::Duration::from_secs(60);
    let edit_throttle = std::time::Duration::from_millis(1500);
    let mut pending_edit = false;

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ToolCall { name, id, args } => {
                let emoji = tool_progress_emoji(&name);
                let desc = describe_tool_call(&name, &args);
                let line = format!("{emoji}  {desc}");
                let idx = progress_lines.len();
                progress_lines.push(line);
                current_tool_ids.insert(id, idx);
                pending_edit = true;
            }
            StreamEvent::ToolResult {
                id,
                is_error,
                ref content,
                ..
            } => {
                if let Some(&idx) = current_tool_ids.get(&id) {
                    if let Some(line) = progress_lines.get_mut(idx) {
                        // Replace the leading emoji with a status indicator.
                        // The line format is "{emoji}  {description}" — find the
                        // double-space separator and keep everything after it.
                        if let Some(sep) = line.find("  ") {
                            let description = &line[sep + 2..]; // 2 bytes for "  "
                            // Extract a brief result summary (first meaningful line)
                            let summary = extract_result_summary(content, 60);
                            if is_error {
                                if summary.is_empty() {
                                    *line = format!("❌  {description}");
                                } else {
                                    *line = format!("❌  {description} — {summary}");
                                }
                            } else if summary.is_empty() {
                                *line = format!("✅  {description}");
                            } else {
                                *line = format!("✅  {description} → {summary}");
                            }
                        }
                    }
                    current_tool_ids.remove(&id);
                    pending_edit = true;
                }
            }
            StreamEvent::TextDelta { content } => {
                final_text.push_str(&content);
            }
            StreamEvent::ApprovalRequired {
                id,
                tool_name,
                tool_args,
                reason,
                risk_level,
            } => {
                send_approval_prompt_shared(
                    &state,
                    &channel_id_owned,
                    &target,
                    &id,
                    &tool_name,
                    &tool_args,
                    &reason,
                    risk_level,
                )
                .await;
            }
            StreamEvent::Done => break,
            StreamEvent::Error { message } => {
                let _ = send_response_shared(
                    &state,
                    &channel_id_owned,
                    &target,
                    &format!("❌ Error: {message}"),
                )
                .await;
                break;
            }
            _ => {}
        }

        // Throttled edit/send of the progress message
        if pending_edit && !progress_lines.is_empty() {
            let now = std::time::Instant::now();
            if now.duration_since(last_edit_time) >= edit_throttle {
                let text = format!("🤖 *Working on it…*\n\n{}", progress_lines.join("\n"));
                match &progress_msg_id {
                    Some(msg_id) => {
                        let _ =
                            edit_channel_message(&state, &channel_id_owned, &target, msg_id, &text)
                                .await;
                    }
                    None => {
                        progress_msg_id = send_channel_message_returning_id(
                            &state,
                            &channel_id_owned,
                            &target,
                            &text,
                        )
                        .await;
                    }
                }
                last_edit_time = now;
                pending_edit = false;
            }
        }
    }

    // Final update of progress message — show all steps as completed
    if !progress_lines.is_empty() {
        let text = format!("🤖 *Done*\n\n{}", progress_lines.join("\n"));
        match &progress_msg_id {
            Some(msg_id) => {
                let _ =
                    edit_channel_message(&state, &channel_id_owned, &target, msg_id, &text).await;
            }
            None => {
                let _ = send_response_shared(&state, &channel_id_owned, &target, &text).await;
            }
        }
    }

    // Stop typing indicator
    typing_handle.abort();

    // Send final response
    if !final_text.is_empty() {
        send_response_shared(&state, &channel_id_owned, &target, &final_text).await?;
    }

    // Ensure streaming task completes cleanly
    match stream_handle.await {
        Ok(Err(e)) => warn!(error = %e, "channel streaming task error"),
        Err(e) if !e.is_cancelled() => warn!(error = %e, "channel streaming task panicked"),
        _ => {}
    }

    Ok(())
}

/// Process a streaming API message — spawned as a concurrent task.
pub(crate) async fn process_stream_message(
    state: SharedAgentState,
    text: String,
    session_id_hint: Option<String>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let session_id = if let Some(ref hint) = session_id_hint {
        if let Ok(id) = hint.parse::<Uuid>() {
            state.sessions.get_or_insert(id, "api", "api_user").await
        } else {
            state.sessions.find_or_create("api", hint).await
        }
    } else {
        state.sessions.create_for_channel("api", "api_user").await
    };

    let _ = tx
        .send(StreamEvent::Session {
            session_id: session_id.to_string(),
        })
        .await;

    let incoming = IncomingMessage {
        id: Uuid::new_v4().to_string(),
        channel: "api".to_string(),
        sender: "api_user".to_string(),
        sender_name: Some("API User".to_string()),
        group: None,
        text: Some(text.clone()),
        attachments: vec![],
        is_mention: false,
        is_reply_to_bot: false,
        metadata: serde_json::Value::Null,
    };

    match process_message_streaming_shared(&state, "api", incoming, &tx, Some(session_id)).await {
        Ok(()) => {
            let _ = tx.send(StreamEvent::Done).await;
        }
        Err(e) => {
            let _ = tx
                .send(StreamEvent::Error {
                    message: e.to_string(),
                })
                .await;
        }
    }
}

/// Detect when the model is being lazy — responding with text that suggests the user
/// should finish the work themselves, instead of actually using tools to complete it.
/// Returns `true` if the response looks like a lazy cop-out.
/// Detect whether the model is stopping prematurely ("being lazy") instead of
/// actually completing the task.
///
/// ## Approach (inspired by Codex / Claude Code patterns)
///
/// Production agents use three main strategies:
/// - **Codex**: Pure structural — only loop if tool calls were emitted. No text analysis.
/// - **Claude Code hooks**: Transcript-level checks (e.g., "did tests run?").
/// - **Ralph Wiggum**: Require explicit `<promise>TASK COMPLETE</promise>` tag.
///
/// We use a **conservative hybrid**: only flag as lazy when the text contains strong
/// deferral language ("you can…", "feel free to…") AND the model has done very little
/// work in this session (low iteration count). If the model has already executed many
/// tool calls and iterations, it's more likely genuinely finished.
///
/// `iteration` is the current loop iteration (0-based).
fn is_lazy_stop(text: &str, iteration: usize) -> bool {
    // Very short responses are never lazy — they're confirmations
    if text.len() < 100 {
        return false;
    }

    let lower = text.to_lowercase();

    // ── Strong completion indicators — if present, trust the model ──
    let completion_signals = [
        "all files created",
        "project is complete",
        "everything is set up and working",
        "all done",
        "finished creating all",
        "built the complete",
        "full implementation",
        "all components created",
        "fully functional",
        "here's what i built",
        "here is what i built",
        "i've created all",
        "i have created all",
        "task complete",
    ];
    if completion_signals.iter().any(|p| lower.contains(*p)) {
        return false;
    }

    // ── Deferral phrases — the model is pushing work to the user ──
    let deferral_phrases = [
        "you can customize",
        "you can further",
        "you can modify",
        "you can adjust",
        "you can extend",
        "you can add more",
        "feel free to",
        "i'll leave",
        "left as an exercise",
        "up to you to",
        "you'll need to",
        "you should create",
        "you would need to",
        "the remaining",
        "repeat this for",
        "do the same for",
        "continue this pattern",
        "follow the same pattern",
        "and so on for",
    ];

    let deferral_count: usize = deferral_phrases
        .iter()
        .filter(|p| lower.contains(**p))
        .count();

    // ── Scaffolding-only — model ran a create-* command and stopped ──
    // Only check in early iterations (< 5) when the model hasn't done much yet
    if iteration < 5 {
        let scaffolding_stops = [
            "has been set up",
            "is now set up",
            "successfully set up",
            "ready for development",
            "you can start developing",
            "you can start building",
            "you can now start",
        ];
        let is_scaffolding = scaffolding_stops.iter().any(|p| lower.contains(*p));
        if is_scaffolding && deferral_count >= 1 {
            return true;
        }
    }

    // ── General laziness — require strong signal ──
    // After many iterations (8+), the model has done real work; need 3+ deferrals
    // In early iterations (< 8), 2+ deferrals is suspicious
    let threshold = if iteration >= 8 { 3 } else { 2 };
    deferral_count >= threshold
}

/// Truncate a tool result to fit within the token budget.
/// Preserves the beginning and end of the content, replacing the middle with a note.
fn truncate_tool_result(content: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return content.to_string(); // 0 = no limit
    }
    let max_chars = max_tokens * 4; // ~4 chars per token
    if content.len() <= max_chars {
        return content.to_string();
    }

    // Keep first 60% and last 20% of allowed chars, replace middle with truncation note
    let head_chars = (max_chars * 6) / 10;
    let tail_chars = (max_chars * 2) / 10;
    let head: String = content.chars().take(head_chars).collect();
    let tail: String = content
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let omitted_chars = content.len() - head_chars - tail_chars;
    let omitted_tokens = omitted_chars / 4;

    format!(
        "{head}\n\n[... truncated {omitted_tokens} tokens ({omitted_chars} chars) to fit context window ...]\n\n{tail}"
    )
}

/// Perform LLM-powered compaction if the context is getting large.
/// Uses the fast_model if available, otherwise the primary model.
async fn maybe_compact_context(
    state: &SharedAgentState,
    session_id: Uuid,
) -> claw_core::Result<bool> {
    let needs_compaction = {
        let mem = state.memory.read().await;
        mem.working.needs_compaction(session_id)
    };

    if !needs_compaction {
        return Ok(false);
    }

    let compaction_data = {
        let mem = state.memory.read().await;
        mem.working.prepare_compaction_request(session_id)
    };

    let (text_to_summarize, messages_to_remove) = match compaction_data {
        Some(data) => data,
        None => return Ok(false),
    };

    info!(session = %session_id, messages = messages_to_remove, "performing LLM-powered context compaction");

    // Use fast model for compaction if available, otherwise primary
    let compaction_model = state
        .config
        .agent
        .fast_model
        .as_deref()
        .unwrap_or(&state.config.agent.model);

    let compaction_prompt = format!(
        "Summarize this conversation history concisely. Preserve:\n\
         - The user's original request and goals\n\
         - Key decisions and outcomes\n\
         - File paths, commands, and technical details that were discussed\n\
         - Any errors encountered and how they were resolved\n\
         - Current state of progress (what's done, what remains)\n\n\
         Keep the summary under 500 words. Be factual and specific.\n\n\
         Conversation to summarize:\n{text_to_summarize}"
    );

    let request = LlmRequest {
        model: compaction_model.to_string(),
        messages: vec![Message::text(Uuid::nil(), Role::User, &compaction_prompt)],
        tools: Arc::new(vec![]),
        system: Some(
            "You are a precise conversation summarizer. Output only the summary, nothing else."
                .to_string(),
        ),
        max_tokens: 2048,
        temperature: 0.3,
        thinking_level: Some("off".to_string()),
        stream: false,
    };

    match state.llm.complete(&request, None).await {
        Ok(response) => {
            let summary = response.message.text_content();
            let mut mem = state.memory.write().await;
            mem.working
                .apply_llm_compaction(session_id, &summary, messages_to_remove);
            let new_token_count = mem.working.token_count(session_id);
            info!(
                session = %session_id,
                compacted_messages = messages_to_remove,
                new_tokens = new_token_count,
                "LLM compaction complete"
            );
            Ok(true)
        }
        Err(e) => {
            // Fallback to naive compaction if LLM fails
            warn!(session = %session_id, error = %e, "LLM compaction failed, using naive compaction");
            let mut mem = state.memory.write().await;
            mem.working.compact(session_id);
            Ok(true)
        }
    }
}

/// Core non-streaming message processing using shared state with fine-grained locks.
pub(crate) async fn process_message_shared(
    state: &SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
    override_session_id: Option<Uuid>,
) -> claw_core::Result<String> {
    // Delegate to the streaming path with a sink channel and collect the final text.
    // This eliminates ~750 lines of near-duplicate logic.
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

    process_message_streaming_shared(state, channel_id, incoming, &tx, override_session_id)
        .await?;

    // Drop the sender so the receiver terminates
    drop(tx);

    // Collect all TextDelta events into the final response string
    let mut final_response = String::new();
    while let Some(event) = rx.recv().await {
        if let StreamEvent::TextDelta { content } = event {
            final_response.push_str(&content);
        }
    }

    Ok(final_response)
}

/// Core streaming message processing using shared state with fine-grained locks.
pub(crate) async fn process_message_streaming_shared(
    state: &SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
    tx: &mpsc::Sender<StreamEvent>,
    override_session_id: Option<Uuid>,
) -> claw_core::Result<()> {
    let target = incoming.group.as_deref().unwrap_or(&incoming.sender);
    let session_id = match override_session_id {
        Some(id) => id,
        None => state.sessions.find_or_create(channel_id, target).await,
    };

    // Store reply context so channel_send_file tool can route to the right channel
    {
        let mut ctx = state.reply_context.lock().await;
        *ctx = Some((channel_id.to_string(), target.to_string()));
    }

    // Store stream tx so sub-agents can forward their events to the parent stream
    {
        let mut stx = state.stream_tx.lock().await;
        *stx = Some(tx.clone());
    }

    let user_text = incoming.text.unwrap_or_default();

    // 1. RECEIVE + RECALL — embed query (before lock) then search memory
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed(&[&user_text]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            _ => None,
        }
    } else {
        None
    };

    let (context_parts, active_goals) = {
        let mut mem = state.memory.write().await;
        let user_msg = Message::text(session_id, Role::User, &user_text);
        mem.working.push(user_msg);
        drop(mem);
        state.sessions.record_message(session_id).await;
        let mem = state.memory.read().await;

        let relevant_episodes = mem.episodic.search(&user_text);

        // Build a combined keyword query from user text for broader matching
        let search_terms = extract_search_keywords(&user_text);

        // Collect facts from multiple search strategies, dedup by category+key
        let mut seen_fact_keys = std::collections::HashSet::new();
        let mut relevant_facts: Vec<String> = Vec::new();

        // Strategy 1: Vector search
        if let Some(ref qemb) = query_embedding {
            for (fact, _score) in mem.semantic.vector_search(qemb, 10) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        // Strategy 2: Word-level keyword search on user text
        for fact in mem.semantic.search(&user_text).iter().take(10) {
            let fk = format!("{}:{}", fact.category, fact.key);
            if seen_fact_keys.insert(fk) {
                relevant_facts.push(format!(
                    "- [{}] {}: {}",
                    fact.category, fact.key, fact.value
                ));
            }
        }

        // Strategy 3: Search with extracted keywords
        if search_terms != user_text.to_lowercase() {
            for fact in mem.semantic.search(&search_terms).iter().take(5) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        relevant_facts.truncate(15);

        let mut parts = Vec::new();
        if !relevant_episodes.is_empty() {
            let episodes_text: Vec<String> = relevant_episodes
                .iter()
                .take(5)
                .map(|e| format!("- {}", e.summary))
                .collect();
            parts.push(format!(
                "Relevant past conversations:\n{}",
                episodes_text.join("\n")
            ));
        }
        if !relevant_facts.is_empty() {
            parts.push(format!(
                "Relevant knowledge:\n{}",
                relevant_facts.join("\n")
            ));
        }

        // Always load learned lessons — these are high-value self-corrections
        let lessons: Vec<String> = mem
            .semantic
            .category("learned_lessons")
            .iter()
            .map(|f| format!("- **{}**: {}", f.key, f.value))
            .collect();
        if !lessons.is_empty() {
            parts.push(format!(
                "Lessons learned from past sessions (apply these!):\n{}",
                lessons.join("\n")
            ));
        }

        drop(mem);

        let planner = state.planner.lock().await;
        let goals: Vec<_> = planner.active_goals().into_iter().cloned().collect();
        drop(planner);

        (parts, goals)
    };

    // 2. BUILD system prompt — no locks needed
    let mut system_prompt = state
        .config
        .agent
        .system_prompt
        .clone()
        .unwrap_or_else(build_default_system_prompt);
    if !context_parts.is_empty() {
        system_prompt.push_str("\n\n<memory>\n");
        system_prompt.push_str(&context_parts.join("\n\n"));
        system_prompt.push_str("\n</memory>");
    }
    if !active_goals.is_empty() {
        system_prompt.push_str("\n\n<active_goals>\n");
        for goal in &active_goals {
            system_prompt.push_str(&format!(
                "- [{}] {} (progress: {:.0}%)\n",
                goal.id,
                goal.description,
                goal.progress * 100.0
            ));
        }
        system_prompt.push_str("</active_goals>");
    }

    // Add mesh peer context so the LLM knows about the network
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() {
            let peers = mesh.peer_list();
            if !peers.is_empty() {
                system_prompt.push_str("\n\n<mesh_network>\n");
                system_prompt.push_str(&format!(
                    "Your peer ID: {}\n",
                    &mesh.peer_id()[..12.min(mesh.peer_id().len())]
                ));
                system_prompt.push_str(&format!(
                    "Your capabilities: [{}]\n",
                    state.config.mesh.capabilities.join(", ")
                ));
                system_prompt.push_str(&format!("Connected peers ({}):\n", peers.len()));
                for p in &peers {
                    system_prompt.push_str(&format!(
                        "  - {} ({}) — capabilities: [{}]\n",
                        p.hostname,
                        &p.peer_id[..8.min(p.peer_id.len())],
                        p.capabilities.join(", "),
                    ));
                }
                system_prompt.push_str(
                    "Use mesh_delegate to send tasks to peers with capabilities you lack.\n",
                );
                system_prompt.push_str("</mesh_network>");
            }
        }
    }

    // Add available skills to system prompt (SKILL.md prompt-injection)
    {
        let skills = state.skills.lock().await;
        if let Some(block) = skills.system_prompt_block() {
            system_prompt.push_str(&block);
        }
    }

    // Add credential provider context so the LLM knows how to retrieve secrets
    if state.config.credentials.provider != "none" {
        system_prompt.push_str("\n\n<credentials>\n");
        system_prompt.push_str(&format!(
            "Provider: {}\n",
            state.config.credentials.provider
        ));
        if let Some(ref vault) = state.config.credentials.default_vault {
            system_prompt.push_str(&format!("Default vault: {vault}\n"));
        }
        let has_service_account = state.config.credentials.service_account_token.is_some();
        if has_service_account {
            system_prompt.push_str(
                "Mode: service account (headless — no biometric prompts)\n\
                 OP_SERVICE_ACCOUNT_TOKEN is set in the environment. The `op` CLI works without the desktop app.\n\
                 You can call `op` commands directly — no Touch ID or user interaction required.\n\n"
            );
        } else {
            system_prompt.push_str(
                "Mode: desktop app integration (biometric / Touch ID)\n\
                 The 1Password desktop app handles authentication via biometric unlock.\n\
                 IMPORTANT: To avoid repeated Touch ID prompts, batch credential lookups using `op run`:\n\
                   export FIELD1=\"op://Vault/Item/field1\"\n\
                   export FIELD2=\"op://Vault/Item/field2\"\n\
                   op run -- sh -c 'echo \"user=$FIELD1 pass=$FIELD2\"'\n\
                 This triggers biometric ONCE for the entire batch instead of per-command.\n\
                 For single lookups, `op read \"op://Vault/Item/field\"` is fine (one prompt).\n\n"
            );
        }
        system_prompt.push_str(
            "When a task requires credentials (passwords, API keys, tokens):\n\
             1. Check memory first with memory_search for the item name/vault mapping\n\
             2. Retrieve the credential using the provider CLI (e.g. `op read \"op://Vault/Item/field\"` or `op item get`)\n\
             3. Use the credential directly — never store the secret itself in memory\n\
             4. After first successful retrieval, store the MAPPING in memory (e.g. \"Plesk credentials → 1Password item 'Plesk Admin' in vault 'Servers'\")\n\
             The operator has pre-configured this provider. Proceed with credential retrieval without asking for permission.\n"
        );
        system_prompt.push_str("</credentials>");
    }

    let mut all_tools_vec = state.tools.tools();
    all_tools_vec.extend(state.plugins.tools());
    all_tools_vec.extend(DeviceTools::tools());
    let all_tools = Arc::new(all_tools_vec);
    state.budget.reset_loop();

    let autonomy_level = AutonomyLevel::from_u8(state.config.autonomy.level);
    let mut iteration = 0;
    let max_iterations = state.config.agent.max_iterations;
    let tool_result_max_tokens = state.config.agent.tool_result_max_tokens;
    let mut consecutive_llm_failures: u32 = 0;

    // Wall-clock deadline for this request
    let started_at = std::time::Instant::now();
    let timeout_secs = state.config.agent.request_timeout_secs;
    let deadline = if timeout_secs > 0 {
        Some(started_at + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };

    // Run serialization — acquire per-session lock to prevent interleaving
    let session_lock = state.sessions.run_lock(session_id).await;
    let _run_guard = session_lock.lock().await;

    // Track tool names from the previous turn to avoid misfiring lazy-stop
    // after legitimate completion (e.g. process_start for dev server).
    let mut last_turn_tool_names: Vec<String> = Vec::new();

    // Configure context window for this session
    let context_window = claw_config::resolve_context_window(
        state.config.agent.context_window,
        &state.config.agent.model,
    );
    {
        let mut mem = state.memory.write().await;
        mem.working.set_context_window(
            session_id,
            context_window,
            state.config.agent.compaction_threshold,
        );
    }

    // 3. THINK + ACT loop with streaming
    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!(session = %session_id, "max agent iterations reached");
            break;
        }

        // Check wall-clock timeout
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            warn!(session = %session_id, elapsed_secs = started_at.elapsed().as_secs(), "request timeout reached in streaming loop");
            let _ = tx.send(StreamEvent::TextDelta {
                    content: format!(
                        "\n\n⏱️ Time limit reached ({}s, {} iterations). Send another message to continue.",
                        timeout_secs, iteration - 1
                    ),
                }).await;
            break;
        }

        state.budget.check()?;

        // Try LLM-powered compaction before reading messages if context is large
        let _ = maybe_compact_context(state, session_id).await;

        // Read messages — brief lock
        let messages = {
            let mem = state.memory.read().await;
            mem.working.messages(session_id).to_vec()
        };

        let request = LlmRequest {
            model: if consecutive_llm_failures >= 3 {
                state
                    .config
                    .agent
                    .fallback_model
                    .as_deref()
                    .unwrap_or(&state.config.agent.model)
                    .to_string()
            } else {
                state.config.agent.model.clone()
            },
            messages,
            tools: all_tools.clone(),
            system: Some(system_prompt.clone()),
            max_tokens: state.config.agent.max_tokens,
            temperature: state.config.agent.temperature,
            thinking_level: Some(state.config.agent.thinking_level.clone()),
            stream: true,
        };

        // Stream from LLM with overflow recovery and model fallback
        let mut chunk_rx = match state
            .llm
            .stream(&request, state.config.agent.fallback_model.as_deref())
            .await
        {
            Ok(rx) => {
                consecutive_llm_failures = 0;
                rx
            }
            Err(ref e)
                if matches!(
                    e,
                    claw_core::ClawError::ContextOverflow { .. }
                        | claw_core::ClawError::LlmProvider(_)
                ) && iteration <= max_iterations =>
            {
                consecutive_llm_failures += 1;
                warn!(session = %session_id, error = %e, consecutive_failures = consecutive_llm_failures,
                    "stream call failed, attempting emergency compaction");
                {
                    let mut mem = state.memory.write().await;
                    mem.working.compact(session_id);
                }
                let messages = {
                    let mem = state.memory.read().await;
                    mem.working.messages(session_id).to_vec()
                };
                let retry_request = LlmRequest {
                    messages,
                    ..request
                };
                state
                    .llm
                    .stream(&retry_request, state.config.agent.fallback_model.as_deref())
                    .await?
            }
            Err(e) => return Err(e),
        };

        let mut full_text = String::new();
        let mut tool_calls: Vec<claw_core::ToolCall> = Vec::new();
        let mut total_usage = claw_llm::Usage::default();
        let mut has_tool_calls = false;
        let mut stop_reason = StopReason::EndTurn;

        // Process stream chunks — no lock needed
        while let Some(chunk) = chunk_rx.recv().await {
            match chunk {
                claw_llm::StreamChunk::TextDelta(text) => {
                    full_text.push_str(&text);
                    let _ = tx.send(StreamEvent::TextDelta { content: text }).await;
                }
                claw_llm::StreamChunk::Thinking(text) => {
                    let _ = tx.send(StreamEvent::Thinking { content: text }).await;
                }
                claw_llm::StreamChunk::ToolCall(tc) => {
                    let _ = tx
                        .send(StreamEvent::ToolCall {
                            name: tc.tool_name.clone(),
                            id: tc.id.clone(),
                            args: tc.arguments.clone(),
                        })
                        .await;
                    tool_calls.push(tc);
                    has_tool_calls = true;
                }
                claw_llm::StreamChunk::Usage(usage) => {
                    total_usage.merge(&usage);
                    let _ = tx
                        .send(StreamEvent::Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            cost_usd: usage.estimated_cost_usd,
                        })
                        .await;
                }
                claw_llm::StreamChunk::Done(reason) => {
                    stop_reason = reason;
                    break;
                }
                claw_llm::StreamChunk::Error(e) => {
                    let _ = tx.send(StreamEvent::Error { message: e }).await;
                    return Ok(());
                }
            }
        }

        state.budget.record_spend(total_usage.estimated_cost_usd)?;

        // Store assistant message — brief lock
        {
            let mut mem = state.memory.write().await;
            let mut assistant_msg = Message::text(session_id, Role::Assistant, &full_text);
            assistant_msg.tool_calls = tool_calls.clone();
            mem.working.push(assistant_msg);
        }
        state.sessions.record_message(session_id).await;

        if !has_tool_calls {
            // Check WHY the model stopped — don't just break blindly
            match stop_reason {
                StopReason::MaxTokens => {
                    // Model was cut off mid-output — inject continuation prompt and loop
                    info!(session = %session_id, iteration, "model hit max_tokens in stream, injecting continuation prompt");
                    let mut mem = state.memory.write().await;
                    let continue_msg = Message::text(
                        session_id,
                        Role::User,
                        "[SYSTEM: Your previous response was truncated because it exceeded the output token limit. \
                         Continue exactly where you left off. Do NOT repeat what you already said or re-explain — \
                         just keep going with the next tool calls or remaining work.]",
                    );
                    mem.working.push(continue_msg);
                    let _ = tx
                        .send(StreamEvent::TextDelta {
                            content: "\n\n*Continuing...*\n\n".to_string(),
                        })
                        .await;
                    continue;
                }
                _ => {
                    // Model chose to stop — check if it's being lazy
                    // BUT: skip lazy-stop if the model just started a dev server
                    // or background process — that's a legitimate final step.
                    let just_started_server = last_turn_tool_names
                        .iter()
                        .any(|name| name == "process_start" || name == "terminal_run")
                        && (full_text.to_lowercase().contains("localhost")
                            || full_text.to_lowercase().contains("running")
                            || full_text.to_lowercase().contains("dev server")
                            || full_text.to_lowercase().contains("started"));
                    if !just_started_server
                        && is_lazy_stop(&full_text, iteration as usize)
                        && iteration < max_iterations
                    {
                        info!(session = %session_id, iteration, "detected lazy model stop in stream, re-prompting");
                        let mut mem = state.memory.write().await;
                        let nudge_msg = Message::text(
                            session_id,
                            Role::User,
                            "[SYSTEM: You stopped but the task is NOT complete. Do NOT describe what could be done — \
                             actually DO it. Use your tools to create the remaining files and finish the job. \
                             Continue working now.]",
                        );
                        mem.working.push(nudge_msg);
                        let _ = tx
                            .send(StreamEvent::TextDelta {
                                content: "\n\n*Continuing...*\n\n".to_string(),
                            })
                            .await;
                        continue;
                    }
                    // Genuinely done
                    break;
                }
            }
        }

        // 4. Execute tool calls with guardrails — parallel when safe
        let parallel_enabled = state.config.agent.parallel_tool_calls;
        let can_parallelize = parallel_enabled && tool_calls.len() > 1;

        if can_parallelize && tool_calls.iter().all(|tc| is_parallel_safe(&tc.tool_name)) {
            // All tool calls are parallel-safe — run them all concurrently
            let mut join_set = tokio::task::JoinSet::new();
            for tool_call in tool_calls.clone() {
                state.budget.record_tool_call()?;
                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });
                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, &tool_call, autonomy_level);
                let s = state.clone();
                let tc = tool_call.clone();
                let tc_id = tool_call.id.clone();
                join_set.spawn(async move {
                    let result = match verdict {
                        GuardrailVerdict::Approve => execute_tool_shared(&s, &tc).await,
                        GuardrailVerdict::Deny(reason) => ToolResult {
                            tool_call_id: tc_id.clone(),
                            content: format!("DENIED: {reason}"),
                            is_error: true,
                            data: None,
                        },
                        _ => execute_tool_shared(&s, &tc).await,
                    };
                    (tc_id, result)
                });
            }

            // Collect results as they complete and stream them back
            while let Some(join_result) = join_set.join_next().await {
                if let Ok((tc_id, tool_result)) = join_result {
                    let _ = tx
                        .send(StreamEvent::ToolResult {
                            id: tc_id.clone(),
                            content: tool_result.content.clone(),
                            is_error: tool_result.is_error,
                            data: tool_result.data.clone(),
                        })
                        .await;
                    let truncated_content =
                        truncate_tool_result(&tool_result.content, tool_result_max_tokens);
                    {
                        let mut mem = state.memory.write().await;
                        let result_msg = Message {
                            id: Uuid::new_v4(),
                            session_id,
                            role: Role::Tool,
                            content: vec![claw_core::MessageContent::ToolResult {
                                tool_call_id: tc_id,
                                content: truncated_content,
                                is_error: tool_result.is_error,
                            }],
                            timestamp: chrono::Utc::now(),
                            tool_calls: vec![],
                            metadata: Default::default(),
                        };
                        mem.working.push(result_msg);
                    }
                }
            }
        } else {
            // Sequential execution (original path)
            for tool_call in &tool_calls {
                state.budget.record_tool_call()?;

                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });

                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, tool_call, autonomy_level);
                let tool_result = match verdict {
                    GuardrailVerdict::Approve => execute_tool_shared(state, tool_call).await,
                    GuardrailVerdict::Deny(reason) => ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        content: format!("DENIED: {reason}"),
                        is_error: true,
                        data: None,
                    },
                    GuardrailVerdict::Escalate(_reason) => {
                        let approval_id = Uuid::new_v4();

                        // Emit approval event to stream so UI can show approve/deny
                        let _ = tx
                            .send(StreamEvent::ApprovalRequired {
                                id: approval_id.to_string(),
                                tool_name: tool_call.tool_name.clone(),
                                tool_args: tool_call.arguments.clone(),
                                reason: _reason.clone(),
                                risk_level: tool_def.risk_level,
                            })
                            .await;

                        // Wait for approval — no lock held during this potentially long wait
                        let response = state
                            .approval
                            .request_approval_with_id(
                                approval_id,
                                &tool_call.tool_name,
                                &tool_call.arguments,
                                &_reason,
                                tool_def.risk_level,
                                120,
                            )
                            .await;
                        match response {
                            ApprovalResponse::Approved => {
                                execute_tool_shared(state, tool_call).await
                            }
                            ApprovalResponse::Denied => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Human denied the action".into(),
                                is_error: true,
                                data: None,
                            },
                            ApprovalResponse::TimedOut => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Approval request timed out".into(),
                                is_error: true,
                                data: None,
                            },
                        }
                    }
                };

                let _ = tx
                    .send(StreamEvent::ToolResult {
                        id: tool_call.id.clone(),
                        content: tool_result.content.clone(),
                        is_error: tool_result.is_error,
                        data: tool_result.data.clone(),
                    })
                    .await;

                // Truncate tool result to fit context window
                let truncated_content =
                    truncate_tool_result(&tool_result.content, tool_result_max_tokens);

                // Store tool result — brief lock
                {
                    let mut mem = state.memory.write().await;
                    let result_msg = Message {
                        id: Uuid::new_v4(),
                        session_id,
                        role: Role::Tool,
                        content: vec![claw_core::MessageContent::ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            content: truncated_content,
                            is_error: tool_result.is_error,
                        }],
                        timestamp: chrono::Utc::now(),
                        tool_calls: vec![],
                        metadata: Default::default(),
                    };
                    mem.working.push(result_msg);
                }
            }
        }

        // Try LLM-powered compaction if context is getting large
        let _ = maybe_compact_context(state, session_id).await;

        // Record this turn's tool names for next iteration's lazy-stop check
        last_turn_tool_names = tool_calls.iter().map(|tc| tc.tool_name.clone()).collect();
    }

    // Auto-resume: if we hit max_iterations or timeout with active goals, schedule a resume
    if state.config.agent.auto_resume {
        let was_interrupted = iteration > max_iterations
            || deadline.is_some_and(|dl| std::time::Instant::now() >= dl);
        if was_interrupted {
            let has_active_goals = {
                let planner = state.planner.lock().await;
                !planner.active_goals().is_empty()
            };
            if has_active_goals && let Some(ref scheduler) = state.scheduler {
                let resume_desc = format!(
                    "Auto-resume: Continue working on unfinished tasks from session {session_id}. \
                         Review active goals with goal_list and continue where you left off."
                );
                let task_id = scheduler
                    .add_one_shot(
                        resume_desc,
                        60, // Resume in 60 seconds
                        Some(format!("auto-resume:{session_id}")),
                        Some(session_id),
                    )
                    .await;
                info!(
                    task_id = %task_id,
                    session = %session_id,
                    "scheduled auto-resume in 60s for interrupted streaming task"
                );
                let _ = tx
                    .send(StreamEvent::TextDelta {
                        content: "\n\n⏱️ I'll automatically resume this work in about 1 minute."
                            .to_string(),
                    })
                    .await;
            }
        }
    }

    // 5. REMEMBER — record episodic memory + audit
    {
        let mut mem = state.memory.write().await;
        mem.audit("message", "processed", Some(&user_text))?;

        // Build a brief summary for episodic memory
        let messages = mem.working.messages(session_id);
        let msg_count = messages.len();
        if msg_count >= 2 {
            // Get last assistant text for summary
            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.text_content())
                .unwrap_or_default();
            let summary = build_episode_summary(messages, &user_text, &last_assistant);
            let episode = claw_memory::episodic::Episode {
                id: uuid::Uuid::new_v4(),
                session_id,
                summary,
                outcome: Some("completed".to_string()),
                tags: extract_episode_tags(&user_text),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            mem.episodic.record(episode);
        }
    }

    // 6. LEARN — extract lessons from error→correction→success patterns
    maybe_extract_lessons(state, session_id).await;

    // Auto-set session label from first user message if not yet set
    if let Some(session) = state.sessions.get(session_id).await
        && session.name.is_none()
        && !user_text.is_empty()
    {
        let label: String = user_text.chars().take(60).collect();
        let label = label
            .split('\n')
            .next()
            .unwrap_or(&label)
            .trim()
            .to_string();
        state.sessions.set_name(session_id, &label).await;
    }

    // Clear reply context — this streaming session is done
    {
        let mut ctx = state.reply_context.lock().await;
        *ctx = None;
    }

    // Clear stream tx
    {
        let mut stx = state.stream_tx.lock().await;
        *stx = None;
    }

    Ok(())
}
