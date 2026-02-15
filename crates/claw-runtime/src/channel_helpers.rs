use tracing::{debug, info, warn};
use uuid::Uuid;

use claw_autonomy::ApprovalResponse;
use claw_channels::adapter::{ApprovalPrompt, OutgoingMessage};

use crate::agent::{PendingApprovals, SharedAgentState};

pub(crate) async fn send_response_shared(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    text: &str,
) -> claw_core::Result<()> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            channel
                .send(OutgoingMessage {
                    channel: channel_id.to_string(),
                    target: target.to_string(),
                    text: text.to_string(),
                    attachments: vec![],
                    reply_to: None,
                })
                .await?;
            return Ok(());
        }
    }
    warn!(channel = channel_id, "channel not found for response");
    Ok(())
}

/// Send a typing indicator to a specific channel target.
pub(crate) async fn send_typing_to_channel(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
) {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            let _ = channel.send_typing(target).await;
            return;
        }
    }
}

/// Map a tool name to an emoji for channel progress messages.
pub(crate) fn tool_progress_emoji(name: &str) -> &'static str {
    if name.starts_with("browser_") {
        return "🌐";
    }
    if name.starts_with("android_") || name.starts_with("ios_") {
        return "📱";
    }
    match name {
        "shell_exec" => "⚡",
        "file_write" | "file_create" | "file_patch" => "📝",
        "file_read" | "directory_list" | "file_list" | "file_find" => "📖",
        "process_start" | "terminal_run" => "🚀",
        "web_search" | "brave_search" => "🔍",
        "memory_store" | "memory_search" | "memory_forget" => "🧠",
        "goal_create" | "goal_update" => "🎯",
        "mesh_delegate" => "🌐",
        "channel_send_file" => "📎",
        _ => "🔧",
    }
}

/// Build a human-readable description of a tool call from its name and arguments.
pub(crate) fn describe_tool_call(name: &str, args: &serde_json::Value) -> String {
    match name {
        "shell_exec" => {
            let cmd = args["command"].as_str().unwrap_or("…");
            // Truncate long commands but keep the first useful part
            let short: String = cmd.chars().take(60).collect();
            if cmd.len() > 60 {
                format!("`{}…`", short.trim())
            } else {
                format!("`{}`", short.trim())
            }
        }
        "file_write" | "file_create" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            // Show just the filename or last 2 path components
            let short = short_path(path);
            format!("Writing `{short}`")
        }
        "file_patch" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            format!("Editing `{}`", short_path(path))
        }
        "file_read" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            format!("Reading `{}`", short_path(path))
        }
        "file_list" | "directory_list" | "file_find" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["directory"].as_str())
                .unwrap_or("…");
            format!("Exploring `{}`", short_path(path))
        }
        "process_start" | "terminal_run" => {
            let cmd = args["command"].as_str().unwrap_or("…");
            let short: String = cmd.chars().take(50).collect();
            format!("Starting `{}`", short.trim())
        }
        "terminal_view" => {
            let id = args["terminal_id"]
                .as_str()
                .or_else(|| args["id"].as_str())
                .unwrap_or("terminal");
            format!("Checking {id}")
        }
        "web_search" | "brave_search" => {
            let q = args["query"].as_str().unwrap_or("…");
            let short: String = q.chars().take(40).collect();
            format!("Searching \"{}\"", short.trim())
        }
        n if n.starts_with("browser_") => {
            let action = n.strip_prefix("browser_").unwrap_or(n);
            match action {
                "navigate" => {
                    let url = args["url"].as_str().unwrap_or("…");
                    let short: String = url.chars().take(50).collect();
                    format!("Opening `{short}`")
                }
                "click" => {
                    let sel = args["selector"].as_str().unwrap_or("element");
                    let short: String = sel.chars().take(30).collect();
                    format!("Clicking `{short}`")
                }
                "type" | "input" => "Typing text".to_string(),
                "screenshot" | "snapshot" => "Taking screenshot".to_string(),
                "upload_file" => "Uploading file".to_string(),
                _ => action.replace('_', " ").to_string(),
            }
        }
        n if n.starts_with("android_") || n.starts_with("ios_") => {
            let parts: Vec<&str> = n.splitn(2, '_').collect();
            let action = parts.get(1).unwrap_or(&n);
            action.replace('_', " ").to_string()
        }
        "memory_store" => "Storing memory".to_string(),
        "memory_search" => "Searching memory".to_string(),
        "goal_create" => {
            let desc = args["description"].as_str().unwrap_or("…");
            let short: String = desc.chars().take(40).collect();
            format!("Goal: {}", short.trim())
        }
        "mesh_delegate" => "Delegating to peer".to_string(),
        "channel_send_file" => {
            let path = args["file_path"].as_str().unwrap_or("…");
            format!("Sending `{}`", short_path(path))
        }
        _ => name.replace('_', " ").to_string(),
    }
}

/// Shorten a file path to the last 2 components (e.g. `src/app/page.tsx`).
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(3).collect();
    let mut result: Vec<&str> = parts.into_iter().rev().collect();
    // Skip empty leading component from absolute paths
    if result.first() == Some(&"") {
        result.remove(0);
    }
    result.join("/")
}

/// Extract a brief, human-readable summary from a tool result.
/// Skips boilerplate lines (Exit code, STDOUT/STDERR headers) and returns
/// the first meaningful line of content, truncated to `max_len` chars.
pub(crate) fn extract_result_summary(content: &str, max_len: usize) -> String {
    let skip = |line: &str| -> bool {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with("Exit code:")
            || trimmed == "STDOUT:"
            || trimmed == "STDERR:"
            || trimmed == "Command completed successfully (no output)."
    };

    for line in content.lines() {
        if !skip(line) {
            let trimmed = line.trim();
            if trimmed.len() > max_len {
                return format!("{}…", &trimmed[..max_len - 1]);
            }
            return trimmed.to_string();
        }
    }
    String::new()
}

/// Send a message through a channel and return its platform message ID.
pub(crate) async fn send_channel_message_returning_id(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    text: &str,
) -> Option<String> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            return channel
                .send_returning_id(OutgoingMessage {
                    channel: channel_id.to_string(),
                    target: target.to_string(),
                    text: text.to_string(),
                    attachments: vec![],
                    reply_to: None,
                })
                .await
                .ok()
                .flatten();
        }
    }
    None
}

/// Edit a previously sent message on a channel.
pub(crate) async fn edit_channel_message(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    message_id: &str,
    text: &str,
) -> claw_core::Result<()> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            return channel.edit_message(target, message_id, text).await;
        }
    }
    Ok(())
}

/// Resolve a pending approval (from callback query, /approve command, or API).
pub(crate) async fn resolve_approval(
    pending: &PendingApprovals,
    id: Uuid,
    approve: bool,
) -> Result<(), String> {
    let tx = pending
        .lock()
        .await
        .remove(&id)
        .ok_or_else(|| "Approval not found or already resolved.".to_string())?;
    let response = if approve {
        ApprovalResponse::Approved
    } else {
        ApprovalResponse::Denied
    };
    let _ = tx.send(response);
    info!(id = %id, approved = approve, "approval resolved via channel");
    Ok(())
}

/// Send an approval prompt to a channel (uses inline keyboard for Telegram, text fallback for others).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_approval_prompt_shared(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    approval_id: &str,
    tool_name: &str,
    tool_args: &serde_json::Value,
    reason: &str,
    risk_level: u8,
) {
    let prompt = ApprovalPrompt {
        approval_id: approval_id.to_string(),
        target: target.to_string(),
        tool_name: tool_name.to_string(),
        tool_args: tool_args.clone(),
        reason: reason.to_string(),
        risk_level,
    };

    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            if let Err(e) = channel.send_approval_prompt(prompt).await {
                warn!(error = %e, channel = channel_id, "failed to send approval prompt");
            }
            return;
        }
    }

    // If the channel isn't found (e.g. "api"), the approval will still work via the API endpoint
    debug!(
        channel = channel_id,
        "no channel found for approval prompt (API-only approval)"
    );
}
