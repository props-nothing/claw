use std::sync::Arc;

use tracing::error;

use claw_runtime::AgentRuntime;

pub(super) async fn cmd_chat(
    config: claw_config::ClawConfig,
    _session: Option<String>,
) -> claw_core::Result<()> {
    println!("🦞 Claw Interactive Chat");
    println!("   Type 'exit' or Ctrl+C to quit");
    println!("   Type '/status' for agent status");
    println!("   Type '/goals' to list goals");
    println!();

    let mut runtime = AgentRuntime::new(config.clone())?;

    // Register LLM providers — config file keys take priority, env vars are fallback
    let mut providers_registered = 0u32;
    if let Some(ref key) = config.services.anthropic_api_key {
        runtime.add_provider(Arc::new(claw_llm::anthropic::AnthropicProvider::new(
            key.clone(),
        )));
        providers_registered += 1;
    }
    if let Some(ref key) = config.services.openai_api_key {
        runtime.add_provider(Arc::new(claw_llm::openai::OpenAiProvider::new(key.clone())));
        providers_registered += 1;
    }

    if providers_registered == 0 {
        let model = &config.agent.model;
        let is_local = model.starts_with("ollama/") || model.starts_with("local/");
        if !is_local {
            eprintln!("⚠️  No LLM API keys found.");
            if model.starts_with("anthropic/") {
                eprintln!("   Add to [services] in claw.toml:  anthropic_api_key = \"sk-ant-...\"");
                eprintln!("   Or set env var: export ANTHROPIC_API_KEY=sk-ant-...");
            } else if model.starts_with("openai/") {
                eprintln!("   Add to [services] in claw.toml:  openai_api_key = \"sk-...\"");
                eprintln!("   Or set env var: export OPENAI_API_KEY=sk-...");
            } else {
                eprintln!(
                    "   Add API keys to [services] in claw.toml or set ANTHROPIC_API_KEY / OPENAI_API_KEY."
                );
            }
            eprintln!();
        }
    }

    // Spawn the runtime in the background
    tokio::spawn(async move {
        if let Err(e) = runtime.run().await {
            error!(error = %e, "runtime exited with error");
        }
    });

    // Wait a moment for the runtime to register its handle
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let handle = claw_runtime::get_runtime_handle().await;
    let handle = match handle {
        Some(h) => h,
        None => {
            println!("❌ Failed to connect to runtime. Is the agent starting up?");
            return Ok(());
        }
    };

    let session_id: Option<String> = _session;

    // Interactive loop reading from stdin
    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    use tokio::io::AsyncBufReadExt;
    let mut lines = reader.lines();

    loop {
        // Print prompt
        eprint!("\x1b[36myou>\x1b[0m ");
        use std::io::Write;
        std::io::stderr().flush().ok();

        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break, // EOF
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "exit" || trimmed == "quit" || trimmed == "/exit" {
            println!("👋 Goodbye!");
            break;
        }

        // Send message to agent via streaming so we can handle approval prompts
        match handle
            .chat_stream(trimmed.to_string(), session_id.clone())
            .await
        {
            Ok(mut rx) => {
                let mut got_text = false;
                while let Some(event) = rx.recv().await {
                    use claw_runtime::StreamEvent;
                    match event {
                        StreamEvent::TextDelta { content } => {
                            if !got_text {
                                eprint!("\x1b[32mclaw>\x1b[0m ");
                                got_text = true;
                            }
                            print!("{content}");
                            std::io::stdout().flush().ok();
                        }
                        StreamEvent::Thinking { content } => {
                            eprint!("\x1b[90m💭 {content}\x1b[0m");
                        }
                        StreamEvent::ToolCall { name, id: _, .. } => {
                            eprintln!("\x1b[33m🔧 Calling tool: {name}\x1b[0m");
                        }
                        StreamEvent::ToolResult {
                            id: _,
                            content,
                            is_error,
                            ..
                        } => {
                            if is_error {
                                eprintln!(
                                    "\x1b[31m   ❌ {}\x1b[0m",
                                    truncate_output(&content, 200)
                                );
                            } else {
                                eprintln!("\x1b[90m   ✓ {}\x1b[0m", truncate_output(&content, 200));
                            }
                        }
                        StreamEvent::ApprovalRequired {
                            id,
                            tool_name,
                            tool_args,
                            reason,
                            risk_level,
                        } => {
                            println!();
                            println!("\x1b[33m⚠️  APPROVAL REQUIRED\x1b[0m");
                            println!("   🔧 Tool: \x1b[1m{tool_name}\x1b[0m");
                            println!("   ⚡ Risk: {risk_level}/10");
                            println!("   📋 Reason: {reason}");
                            let args_pretty = serde_json::to_string_pretty(&tool_args)
                                .unwrap_or_else(|_| tool_args.to_string());
                            let args_short = truncate_output(&args_pretty, 300);
                            println!("\x1b[90m   {args_short}\x1b[0m");
                            println!();
                            eprint!("\x1b[33m   Approve? [y/n]>\x1b[0m ");
                            std::io::stderr().flush().ok();

                            // Read the approval decision
                            let decision = lines.next_line().await;
                            match decision {
                                Ok(Some(ans)) => {
                                    let ans = ans.trim().to_lowercase();
                                    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
                                        if ans == "y" || ans == "yes" || ans == "approve" {
                                            match handle.approve(uuid).await {
                                                Ok(()) => {
                                                    eprintln!("\x1b[32m   ✅ Approved\x1b[0m")
                                                }
                                                Err(e) => {
                                                    eprintln!("\x1b[31m   ❌ {e}\x1b[0m")
                                                }
                                            }
                                        } else {
                                            match handle.deny(uuid).await {
                                                Ok(()) => {
                                                    eprintln!("\x1b[31m   ❌ Denied\x1b[0m")
                                                }
                                                Err(e) => {
                                                    eprintln!("\x1b[31m   ❌ {e}\x1b[0m")
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    // EOF or error — deny by default
                                    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
                                        let _ = handle.deny(uuid).await;
                                        eprintln!("\x1b[31m   ❌ Denied (no input)\x1b[0m");
                                    }
                                }
                            }
                        }
                        StreamEvent::Usage {
                            input_tokens,
                            output_tokens,
                            cost_usd,
                        } => {
                            eprintln!(
                                "\n\x1b[90m   [{input_tokens} in / {output_tokens} out, ${cost_usd:.4}]\x1b[0m"
                            );
                        }
                        StreamEvent::Error { message } => {
                            println!("\x1b[31m❌ Error: {message}\x1b[0m");
                        }
                        StreamEvent::Done => {
                            if got_text {
                                println!(); // newline after streaming text
                            }
                        }
                        StreamEvent::Session { .. } => {}
                    }
                }
            }
            Err(e) => {
                println!("\x1b[31m❌ {e}\x1b[0m");
            }
        }
        println!();
    }

    Ok(())
}

fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.replace('\n', " ")
    } else {
        format!("{}...", &s[..max].replace('\n', " "))
    }
}
