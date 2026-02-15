use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use claw_channels::adapter::IncomingMessage;
use claw_core::{ToolCall, ToolResult};

use crate::agent::{SharedAgentState, StreamEvent, SubTaskState, SubTaskStatus};
use crate::agent_loop::{process_api_message, process_message_streaming_shared};

fn sub_agent_system_prompt(role: &str) -> String {
    let role_instruction = match role {
        "planner" => {
            "You are a planning agent. Your job is to analyze the task, break it down into clear steps, identify required files and dependencies, and create a detailed project plan. Output a structured plan with specific file paths, technologies, and implementation order. Do NOT write code — only plan."
        }
        "coder" | "developer" => {
            "You are a coding agent. Your job is to write production-quality code based on the task description. When research findings are provided from a preceding agent, use them thoroughly — match the structure, content, styling, and details described. Write complete, well-structured files with proper imports, error handling, and documentation. Use your tools to create files and test that they compile/run correctly. After writing code, run the build and fix any errors before finishing."
        }
        "reviewer" => {
            "You are a code review agent. Your job is to review the code that was written, check for bugs, security issues, missing error handling, and style problems. Run tests and linters if available. Report issues clearly with file paths and line numbers. Suggest specific fixes."
        }
        "tester" | "qa" => {
            "You are a testing agent. Your job is to write and run tests for the code. Create unit tests, integration tests, and end-to-end tests as appropriate. Verify that the application works correctly. Report any failures with details."
        }
        "researcher" => {
            "You are a research agent. Your job is to gather information needed for the task. When given URLs, ALWAYS fetch them with http_fetch first — read the actual content, structure, and details before summarizing. Search the web, read documentation, find examples, and compile your findings into a clear, structured summary. Focus on finding practical, actionable information. For website rebuilds: extract page structure, navigation items, section headings, key copy, feature lists, and design notes."
        }
        "devops" | "deployer" => {
            "You are a DevOps agent. Your job is to set up build systems, CI/CD, deployment configurations, Docker files, and infrastructure. Ensure the project can be built, tested, and deployed reliably."
        }
        "debugger" | "fixer" => {
            "You are a debugging agent. Your job is to find and fix errors in the code. Read error messages carefully, trace the root cause, and apply fixes. Run the code again to verify the fix works."
        }
        _ => "You are a specialized agent. Execute the assigned task thoroughly using your tools.",
    };

    format!(
        "You are a Claw 🦞 sub-agent with the role: {role}.\n\n{role_instruction}\n\n\
         Work autonomously — complete the task using your tools without asking for clarification.\n\
         When done, output a clear summary of what you accomplished and any important findings."
    )
}

/// Spawn a sub-agent to work on a task concurrently.
pub(crate) async fn exec_sub_agent_spawn(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let role = match call.arguments.get("role").and_then(|v| v.as_str()) {
        Some(r) => r.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'role' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let task = match call.arguments.get("task").and_then(|v| v.as_str()) {
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

    let context_summary = call
        .arguments
        .get("context_summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let depends_on: Vec<Uuid> = call
        .arguments
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    let _model_override = call
        .arguments
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Optional goal/step linking — auto-complete goal step when sub-agent finishes
    let goal_id: Option<Uuid> = call
        .arguments
        .get("goal_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok());

    let step_id: Option<Uuid> = call
        .arguments
        .get("step_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok());

    let task_id = Uuid::new_v4();
    let parent_session_id = Uuid::new_v4();

    // Determine initial status based on dependencies
    let initial_status = if depends_on.is_empty() {
        SubTaskStatus::Pending
    } else {
        SubTaskStatus::WaitingForDeps
    };

    // If linked to a goal step, mark it as in-progress in the planner
    if let (Some(gid), Some(sid)) = (goal_id, step_id) {
        let mut planner = state.planner.lock().await;
        planner.assign_to_sub_agent(gid, sid, task_id, Some(role.clone()));
        info!(task_id = %task_id, goal_id = %gid, step_id = %sid, "linked sub-agent to goal step");
    }

    // Register the sub-task
    {
        let sub_task_state = SubTaskState {
            task_id,
            role: role.clone(),
            task_description: task.clone(),
            status: initial_status,
            result: None,
            error: None,
            parent_session_id,
            depends_on: depends_on.clone(),
            created_at: std::time::Instant::now(),
            goal_id,
            step_id,
        };
        state
            .pending_sub_tasks
            .lock()
            .await
            .insert(task_id, sub_task_state);
    }

    // Spawn the sub-agent task (uses boxed future to break async type cycle)
    let s = state.clone();
    tokio::spawn(run_sub_agent_task(
        s,
        task_id,
        role.clone(),
        task.clone(),
        context_summary,
        depends_on.clone(),
    ));

    info!(
        task_id = %task_id,
        role = %role,
        deps = ?depends_on,
        "spawned sub-agent"
    );

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Sub-agent spawned successfully.\n\
             Task ID: {}\n\
             Role: {}\n\
             Status: {}\n\
             Dependencies: {}\n\n\
             Use sub_agent_wait with this task_id to collect the result when ready.",
            task_id,
            role,
            if depends_on.is_empty() {
                "running"
            } else {
                "waiting for dependencies"
            },
            if depends_on.is_empty() {
                "none".to_string()
            } else {
                depends_on
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
        is_error: false,
        data: Some(serde_json::json!({
            "task_id": task_id.to_string(),
            "role": role,
            "status": if depends_on.is_empty() { "running" } else { "waiting_for_deps" },
        })),
    }
}

/// Internal: run the sub-agent task through a fresh agent loop.
/// Returns a boxed future to break the async type recursion cycle
/// (process_message_shared → exec_sub_agent_spawn → run_sub_agent_task → process_api_message → process_message_shared).
///
/// If a parent stream_tx is available, sub-agent events (tool calls, results, progress)
/// are forwarded to the parent stream so they render in the web UI and channel outputs.
fn run_sub_agent_task(
    state: SharedAgentState,
    task_id: Uuid,
    role: String,
    task_description: String,
    context_summary: Option<String>,
    depends_on: Vec<Uuid>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        // Wait for dependencies if needed
        let effective_task = if !depends_on.is_empty() {
            info!(task_id = %task_id, deps = ?depends_on, "sub-agent waiting for dependencies");
            loop {
                let all_done = {
                    let tasks = state.pending_sub_tasks.lock().await;
                    depends_on.iter().all(|dep_id| {
                        tasks
                            .get(dep_id)
                            .map(|t| {
                                t.status == SubTaskStatus::Completed
                                    || t.status == SubTaskStatus::Failed
                            })
                            .unwrap_or(true)
                    })
                };
                if all_done {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }

            // Collect dependency results to provide context
            let dep_results: Vec<String> = {
                let tasks = state.pending_sub_tasks.lock().await;
                depends_on
                    .iter()
                    .filter_map(|dep_id| {
                        tasks.get(dep_id).map(|t| {
                            format!(
                                "[{} agent ({})] {}",
                                t.role,
                                if t.status == SubTaskStatus::Completed {
                                    "completed"
                                } else {
                                    "failed"
                                },
                                t.result
                                    .as_deref()
                                    .or(t.error.as_deref())
                                    .unwrap_or("no output")
                            )
                        })
                    })
                    .collect()
            };

            // Build the task message with dependency results
            let mut full_task = task_description.clone();
            if !dep_results.is_empty() {
                full_task.push_str("\n\n## Results from preceding agents:\n");
                for dr in &dep_results {
                    full_task.push_str(dr);
                    full_task.push('\n');
                }
            }
            full_task
        } else {
            task_description.clone()
        };

        // Update status to running
        {
            let mut tasks = state.pending_sub_tasks.lock().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                t.status = SubTaskStatus::Running;
            }
        }

        // Create a fresh session for this sub-agent
        let session_id = state.sessions.create().await;
        let label = format!("sub-agent:{role}");
        state.sessions.set_name(session_id, &label).await;

        // Build the task message with context
        let mut prompt = String::new();
        if let Some(ref ctx) = context_summary {
            prompt.push_str("## Context from parent agent:\n");
            prompt.push_str(ctx);
            prompt.push_str("\n\n");
        }
        prompt.push_str("## Your Task:\n");
        prompt.push_str(&effective_task);

        // Inject role-specific system prompt
        let mut sub_state = state.clone();
        let mut sub_config = sub_state.config.clone();
        sub_config.agent.system_prompt = Some(sub_agent_system_prompt(&role));
        if sub_config.agent.max_iterations < 100 {
            sub_config.agent.max_iterations = 100;
        }
        sub_state.config = sub_config;

        // Check if we have a parent stream tx to forward events to
        let parent_tx = {
            let stx = state.stream_tx.lock().await;
            stx.clone()
        };

        let (result_text, result_error) = if let Some(ref ptx) = parent_tx {
            // Use streaming path — forward sub-agent events to parent stream
            let role_tag = role.clone();

            // Send a marker so the UI/channel knows a sub-agent started
            let _ = ptx
                .send(StreamEvent::TextDelta {
                    content: format!("\n\n🤖 *Sub-agent ({role_tag}) working…*\n"),
                })
                .await;

            // Create a local stream that forwards events to the parent
            let (sub_tx, mut sub_rx) = mpsc::channel::<StreamEvent>(128);
            let ptx_fwd = ptx.clone();
            let role_fwd = role_tag.clone();
            let forwarder = tokio::spawn(async move {
                let mut sub_text = String::new();
                while let Some(event) = sub_rx.recv().await {
                    match event {
                        StreamEvent::ToolCall { name, id, args } => {
                            // Forward tool calls with sub-agent prefix in the name
                            let prefixed_name = format!("[{role_fwd}] {name}");
                            let _ = ptx_fwd
                                .send(StreamEvent::ToolCall {
                                    name: prefixed_name,
                                    id,
                                    args,
                                })
                                .await;
                        }
                        StreamEvent::ToolResult {
                            id,
                            content,
                            is_error,
                            data,
                        } => {
                            // Forward tool results as-is (they match by id)
                            let _ = ptx_fwd
                                .send(StreamEvent::ToolResult {
                                    id,
                                    content,
                                    is_error,
                                    data,
                                })
                                .await;
                        }
                        StreamEvent::TextDelta { content } => {
                            sub_text.push_str(&content);
                        }
                        StreamEvent::Error { message } => {
                            let _ = ptx_fwd
                                .send(StreamEvent::TextDelta {
                                    content: format!(
                                        "\n⚠️ Sub-agent ({role_fwd}) error: {message}\n"
                                    ),
                                })
                                .await;
                        }
                        StreamEvent::Done => break,
                        _ => {} // Skip session, usage, etc.
                    }
                }
                sub_text
            });

            // Build incoming message for the streaming path
            let incoming = IncomingMessage {
                id: Uuid::new_v4().to_string(),
                channel: "sub-agent".to_string(),
                sender: format!("sub-agent:{role_tag}"),
                sender_name: Some(format!("Sub-agent ({role_tag})")),
                group: None,
                text: Some(prompt.clone()),
                attachments: vec![],
                is_mention: false,
                is_reply_to_bot: false,
                metadata: serde_json::Value::Null,
            };

            // Clear the parent stream_tx in sub_state so nested sub-agents
            // don't double-forward (they'll get their own copy if needed)
            {
                let mut stx = sub_state.stream_tx.lock().await;
                *stx = Some(sub_tx.clone());
            }

            let stream_result = process_message_streaming_shared(
                &sub_state,
                "sub-agent",
                incoming,
                &sub_tx,
                Some(session_id),
            )
            .await;

            let _ = sub_tx.send(StreamEvent::Done).await;
            drop(sub_tx);

            // Wait for forwarder to finish and get the accumulated text
            let sub_final_text = forwarder.await.unwrap_or_default();

            // Send completion marker
            let _ = ptx
                .send(StreamEvent::TextDelta {
                    content: format!("\n✅ *Sub-agent ({role_tag}) done*\n\n"),
                })
                .await;

            match stream_result {
                Ok(()) => (sub_final_text, None),
                Err(e) => (sub_final_text, Some(e.to_string())),
            }
        } else {
            // No parent stream — fall back to non-streaming API path
            let result = process_api_message(sub_state, prompt, Some(session_id.to_string())).await;
            (result.text, result.error)
        };

        // Update the sub-task state with the result
        let (is_error, goal_link) = {
            let mut tasks = state.pending_sub_tasks.lock().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                if result_error.is_some() {
                    t.status = SubTaskStatus::Failed;
                    t.error = result_error.clone();
                    t.result = Some(result_text.clone());
                    (true, (t.goal_id, t.step_id))
                } else {
                    t.status = SubTaskStatus::Completed;
                    t.result = Some(result_text.clone());
                    (false, (t.goal_id, t.step_id))
                }
            } else {
                (false, (None, None))
            }
        };

        // Auto-update linked goal step if goal_id/step_id were provided
        if let (Some(gid), Some(sid)) = goal_link {
            let mut planner = state.planner.lock().await;
            if is_error {
                let err_msg = result_error.unwrap_or_else(|| "Sub-agent failed".into());
                let updated = planner.fail_sub_agent_task(task_id, err_msg.clone());
                if updated {
                    info!(task_id = %task_id, role = %role, "auto-failed linked goal step");
                    // Persist updated goal + step to SQLite
                    if let Some(goal) = planner.get(gid) {
                        let mem = state.memory.lock().await;
                        let _ = mem.persist_goal(
                            &gid,
                            &goal.description,
                            &format!("{:?}", goal.status).to_lowercase(),
                            goal.priority,
                            goal.progress,
                            None,
                        );
                        let _ = mem.persist_goal_step(&sid, &gid, "", "failed", Some(&err_msg));
                    }
                }
            } else {
                let summary = result_text.chars().take(500).collect::<String>();
                let updated = planner.complete_sub_agent_task(task_id, summary.clone());
                if updated {
                    info!(task_id = %task_id, role = %role, "auto-completed linked goal step");
                    // Persist updated goal + step to SQLite
                    if let Some(goal) = planner.get(gid) {
                        let mem = state.memory.lock().await;
                        let _ = mem.persist_goal(
                            &gid,
                            &goal.description,
                            &format!("{:?}", goal.status).to_lowercase(),
                            goal.priority,
                            goal.progress,
                            None,
                        );
                        let _ = mem.persist_goal_step(&sid, &gid, "", "completed", Some(&summary));
                    }
                }
            }
        }

        info!(task_id = %task_id, role = %role, "sub-agent task completed");
    })
}

/// Wait for one or more sub-agent tasks to complete.
pub(crate) async fn exec_sub_agent_wait(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_ids: Vec<Uuid> = call
        .arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    if task_ids.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: 'task_ids' must contain at least one task ID".into(),
            is_error: true,
            data: None,
        };
    }

    let timeout_secs = call
        .arguments
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let started = std::time::Instant::now();

    // Poll until all tasks are done
    loop {
        let all_done = {
            let tasks = state.pending_sub_tasks.lock().await;
            task_ids.iter().all(|id| {
                tasks
                    .get(id)
                    .map(|t| {
                        t.status == SubTaskStatus::Completed || t.status == SubTaskStatus::Failed
                    })
                    .unwrap_or(true)
            })
        };

        if all_done {
            break;
        }

        // Check timeout
        if timeout_secs > 0 && started.elapsed().as_secs() >= timeout_secs {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Timeout: waited {timeout_secs}s but not all sub-agent tasks completed."
                ),
                is_error: true,
                data: None,
            };
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    // Collect results
    let tasks = state.pending_sub_tasks.lock().await;
    let mut results = Vec::new();
    let mut result_data = Vec::new();

    for id in &task_ids {
        if let Some(t) = tasks.get(id) {
            let status_str = match t.status {
                SubTaskStatus::Completed => "completed",
                SubTaskStatus::Failed => "failed",
                _ => "unknown",
            };
            results.push(format!(
                "## {} agent [{}] — {}\n{}",
                t.role,
                id,
                status_str,
                t.result
                    .as_deref()
                    .or(t.error.as_deref())
                    .unwrap_or("no output"),
            ));
            result_data.push(serde_json::json!({
                "task_id": id.to_string(),
                "role": t.role,
                "status": status_str,
                "result": t.result,
                "error": t.error,
            }));
        } else {
            results.push(format!("## Task {id} — not found"));
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: results.join("\n\n"),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": result_data })),
    }
}

/// Check the status of sub-agent tasks without blocking.
pub(crate) async fn exec_sub_agent_status(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_ids: Vec<Uuid> = call
        .arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    let tasks = state.pending_sub_tasks.lock().await;

    let entries: Vec<&SubTaskState> = if task_ids.is_empty() {
        tasks.values().collect()
    } else {
        task_ids.iter().filter_map(|id| tasks.get(id)).collect()
    };

    if entries.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No sub-agent tasks found.".into(),
            is_error: false,
            data: None,
        };
    }

    let mut lines = vec![format!("Sub-agent tasks ({}):", entries.len())];
    let mut data = Vec::new();

    for t in &entries {
        let status_str = match t.status {
            SubTaskStatus::WaitingForDeps => "waiting_for_deps",
            SubTaskStatus::Pending => "pending",
            SubTaskStatus::Running => "running",
            SubTaskStatus::Completed => "completed",
            SubTaskStatus::Failed => "failed",
        };
        let elapsed = t.created_at.elapsed().as_secs();
        lines.push(format!(
            "  • {} ({}) — {} [{}s elapsed]{}",
            t.role,
            &t.task_id.to_string()[..8],
            status_str,
            elapsed,
            if let Some(ref r) = t.result {
                format!(" — result: {}...", &r[..r.len().min(100)])
            } else {
                String::new()
            }
        ));
        data.push(serde_json::json!({
            "task_id": t.task_id.to_string(),
            "role": t.role,
            "status": status_str,
            "elapsed_secs": elapsed,
            "has_result": t.result.is_some(),
            "depends_on": t.depends_on.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        }));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": data })),
    }
}

// ─── Scheduler Tool Implementation ─────────────────────────────────────────

/// Persist a ScheduledTask to the memory database.
pub(crate) fn persist_task_to_db(
    mem: &claw_memory::MemoryStore,
    task: &crate::scheduler::ScheduledTask,
) {
    let kind_json = serde_json::to_string(&task.kind).unwrap_or_default();
    let created_at = task.created_at.to_rfc3339();
    let last_fired = task.last_fired.map(|t| t.to_rfc3339());
    if let Err(e) = mem.persist_scheduled_task(
        &task.id.to_string(),
        task.label.as_deref(),
        &task.description,
        &kind_json,
        &created_at,
        task.session_id.as_ref().map(|s| s.to_string()).as_deref(),
        task.active,
        task.fire_count,
        last_fired.as_deref(),
    ) {
        warn!(task_id = %task.id, error = %e, "failed to persist scheduled task to DB");
    }
}

/// Schedule a recurring cron or one-shot delayed task.
pub(crate) async fn exec_cron_schedule(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let description = match call.arguments.get("description").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'description' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let cron_expr = call
        .arguments
        .get("cron_expr")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let delay_seconds = call.arguments.get("delay_seconds").and_then(|v| v.as_u64());
    let label = call
        .arguments
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    if let Some(cron) = cron_expr {
        // Recurring cron task
        match scheduler
            .add_cron(description.clone(), &cron, label.clone(), None)
            .await
        {
            Ok(task_id) => {
                // Persist to SQLite
                if let Some(task) = scheduler.get(task_id).await {
                    let mem = state.memory.lock().await;
                    persist_task_to_db(&mem, &task);
                }
                ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Recurring task scheduled.\n\
                         Task ID: {}\n\
                         Cron: {}\n\
                         Label: {}\n\
                         Description: {}",
                        task_id,
                        cron,
                        label.unwrap_or_else(|| "none".to_string()),
                        description,
                    ),
                    is_error: false,
                    data: Some(serde_json::json!({
                        "task_id": task_id.to_string(),
                        "type": "cron",
                        "cron_expr": cron,
                    })),
                }
            }
            Err(e) => ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error scheduling cron task: {e}"),
                is_error: true,
                data: None,
            },
        }
    } else if let Some(delay) = delay_seconds {
        // One-shot delayed task
        let task_id = scheduler
            .add_one_shot(description.clone(), delay, label.clone(), None)
            .await;
        // Persist to SQLite
        if let Some(task) = scheduler.get(task_id).await {
            let mem = state.memory.lock().await;
            persist_task_to_db(&mem, &task);
        }
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!(
                "One-shot task scheduled.\n\
                 Task ID: {}\n\
                 Fires in: {}s\n\
                 Label: {}\n\
                 Description: {}",
                task_id,
                delay,
                label.unwrap_or_else(|| "none".to_string()),
                description,
            ),
            is_error: false,
            data: Some(serde_json::json!({
                "task_id": task_id.to_string(),
                "type": "one_shot",
                "delay_seconds": delay,
            })),
        }
    } else {
        ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: must provide either 'cron_expr' or 'delay_seconds'".into(),
            is_error: true,
            data: None,
        }
    }
}

pub(crate) async fn exec_cron_list(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let tasks = scheduler.list_all().await;
    if tasks.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No scheduled tasks.".into(),
            is_error: false,
            data: Some(serde_json::json!({ "tasks": [] })),
        };
    }

    let mut lines = Vec::new();
    let mut active_count = 0u32;
    let mut json_tasks = Vec::new();
    for task in &tasks {
        let kind_str = match &task.kind {
            crate::scheduler::ScheduleKind::Cron { expression } => format!("cron: {expression}"),
            crate::scheduler::ScheduleKind::OneShot { fire_at } => {
                format!("one-shot: {}", fire_at.format("%Y-%m-%d %H:%M:%S UTC"))
            }
        };
        let status = if task.active { "active" } else { "inactive" };
        if task.active {
            active_count += 1;
        }
        let label_str = task.label.as_deref().unwrap_or("(none)");
        lines.push(format!(
            "• [{}] {} | {} | label: {} | fires: {} | desc: {}",
            status, task.id, kind_str, label_str, task.fire_count, task.description
        ));
        json_tasks.push(serde_json::json!({
            "id": task.id.to_string(),
            "label": task.label,
            "description": task.description,
            "kind": kind_str,
            "active": task.active,
            "fire_count": task.fire_count,
            "last_fired": task.last_fired.map(|t| t.to_rfc3339()),
            "created_at": task.created_at.to_rfc3339(),
        }));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "{} task(s) ({} active):\n{}",
            tasks.len(),
            active_count,
            lines.join("\n")
        ),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": json_tasks })),
    }
}

pub(crate) async fn exec_cron_cancel(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_id_str = match call.arguments.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'task_id' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let task_id = match uuid::Uuid::parse_str(task_id_str) {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error: invalid UUID: {task_id_str}"),
                is_error: true,
                data: None,
            };
        }
    };

    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let removed = scheduler.remove(task_id).await;
    if removed {
        // Remove from SQLite
        let mem = state.memory.lock().await;
        let _ = mem.delete_scheduled_task(&task_id.to_string());
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Task {task_id} cancelled and removed."),
            is_error: false,
            data: Some(serde_json::json!({ "task_id": task_id.to_string(), "removed": true })),
        }
    } else {
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Task {task_id} not found."),
            is_error: true,
            data: None,
        }
    }
}
