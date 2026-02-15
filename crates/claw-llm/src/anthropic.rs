use async_trait::async_trait;
use claw_core::Result;
use reqwest::Client;
use tracing::{debug, info};

use crate::provider::*;

/// Anthropic Claude API provider.
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com/v1".into(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn build_request_body(&self, request: &LlmRequest) -> serde_json::Value {
        let mut messages = Vec::new();
        for msg in &request.messages {
            match msg.role {
                claw_core::Role::System => continue, // handled via top-level "system" field
                claw_core::Role::User => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.text_content(),
                    }));
                }
                claw_core::Role::Assistant => {
                    if msg.tool_calls.is_empty() {
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": msg.text_content(),
                        }));
                    } else {
                        // Assistant message with tool_use blocks
                        let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                        let text = msg.text_content();
                        if !text.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": text,
                            }));
                        }
                        for tc in &msg.tool_calls {
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.tool_name,
                                "input": tc.arguments,
                            }));
                        }
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content_blocks,
                        }));
                    }
                }
                claw_core::Role::Tool => {
                    // Tool results sent as user message with tool_result content blocks
                    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                    for block in &msg.content {
                        if let claw_core::MessageContent::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                        } = block
                        {
                            content_blocks.push(serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tool_call_id,
                                "content": content,
                                "is_error": is_error,
                            }));
                        }
                    }
                    if content_blocks.is_empty() {
                        // Fallback: send as plain user message
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": msg.text_content(),
                        }));
                    } else {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": content_blocks,
                        }));
                    }
                }
            }
        }

        let mut body = serde_json::json!({
            "model": &request.model,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "messages": messages,
        });

        if let Some(ref system) = request.system {
            body["system"] = serde_json::json!(system);
        }

        // Tool definitions
        if !request.tools.is_empty() {
            let tools: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters,
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        // Extended thinking
        if let Some(ref level) = request.thinking_level {
            if level != "off" {
                let budget = match level.as_str() {
                    "low" => 2048,
                    "medium" => 8192,
                    "high" => 16384,
                    "xhigh" => 32768,
                    _ => 8192,
                };
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget,
                });
            }
        }

        body
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<String> {
        vec![
            "claude-opus-4-6".into(),
            "claude-opus-4-20250514".into(),
            "claude-sonnet-4-20250514".into(),
            "claude-haiku-3-5".into(),
        ]
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let body = self.build_request_body(request);
        debug!(model = %request.model, "sending Anthropic API request");

        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2024-10-22")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 429 {
                return Err(claw_core::ClawError::RateLimited {
                    retry_after_secs: 30,
                });
            }
            return Err(claw_core::ClawError::LlmProvider(format!(
                "HTTP {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

        // Parse the response into our standard format
        let content_text = data["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| {
                        if b["type"] == "text" {
                            b["text"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        // Parse tool calls
        let tool_calls: Vec<claw_core::ToolCall> = data["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| {
                        if b["type"] == "tool_use" {
                            Some(claw_core::ToolCall {
                                id: b["id"].as_str().unwrap_or("").to_string(),
                                tool_name: b["name"].as_str().unwrap_or("").to_string(),
                                arguments: b["input"].clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let has_tool_calls = !tool_calls.is_empty();

        let stop_reason = match data["stop_reason"].as_str() {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        let usage_data = &data["usage"];
        let input_tokens = usage_data["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage_data["output_tokens"].as_u64().unwrap_or(0) as u32;

        let mut message =
            claw_core::Message::text(uuid::Uuid::nil(), claw_core::Role::Assistant, content_text);
        message.tool_calls = tool_calls;

        Ok(LlmResponse {
            message,
            usage: Usage {
                input_tokens,
                output_tokens,
                thinking_tokens: 0,
                cache_read_tokens: usage_data["cache_read_input_tokens"].as_u64().unwrap_or(0)
                    as u32,
                cache_write_tokens: usage_data["cache_creation_input_tokens"]
                    .as_u64()
                    .unwrap_or(0) as u32,
                estimated_cost_usd: estimate_anthropic_cost(
                    &request.model,
                    input_tokens,
                    output_tokens,
                ),
            },
            has_tool_calls,
            stop_reason,
        })
    }

    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let mut body = self.build_request_body(request);
        body["stream"] = serde_json::json!(true);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let model = request.model.clone();

        tokio::spawn(async move {
            let resp = client
                .post(format!("{base_url}/messages"))
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2024-10-22")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(resp) if resp.status().is_success() => {
                    use futures::StreamExt;
                    let mut stream = resp.bytes_stream();
                    let mut buffer = String::new();
                    // Track tool use blocks: index -> (id, name, input_json)
                    let mut current_tool_id = String::new();
                    let mut current_tool_name = String::new();
                    let mut current_tool_input = String::new();
                    let mut in_tool_input = false;
                    let mut input_tokens = 0u32;
                    let mut output_tokens = 0u32;
                    let mut stop_reason = StopReason::EndTurn;
                    let mut has_tool_calls = false;

                    while let Some(chunk_result) = stream.next().await {
                        match chunk_result {
                            Ok(bytes) => {
                                buffer.push_str(&String::from_utf8_lossy(&bytes));
                                // Process complete SSE lines
                                while let Some(newline_pos) = buffer.find('\n') {
                                    let line = buffer[..newline_pos].trim().to_string();
                                    buffer = buffer[newline_pos + 1..].to_string();

                                    if line.is_empty() || line.starts_with(':') {
                                        continue;
                                    }
                                    let Some(data) = line.strip_prefix("data: ") else {
                                        // Could be "event: ..." line — skip
                                        continue;
                                    };
                                    let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                                    else {
                                        continue;
                                    };

                                    match event["type"].as_str() {
                                        Some("message_start") => {
                                            // Extract usage from message_start
                                            if let Some(usage) =
                                                event["message"]["usage"].as_object()
                                            {
                                                if let Some(it) = usage
                                                    .get("input_tokens")
                                                    .and_then(|v| v.as_u64())
                                                {
                                                    input_tokens = it as u32;
                                                }
                                            }
                                        }
                                        Some("content_block_start") => {
                                            let cb = &event["content_block"];
                                            if cb["type"].as_str() == Some("tool_use") {
                                                current_tool_id =
                                                    cb["id"].as_str().unwrap_or("").to_string();
                                                current_tool_name =
                                                    cb["name"].as_str().unwrap_or("").to_string();
                                                current_tool_input.clear();
                                                in_tool_input = true;
                                            }
                                        }
                                        Some("content_block_delta") => {
                                            let delta = &event["delta"];
                                            match delta["type"].as_str() {
                                                Some("text_delta") => {
                                                    if let Some(text) = delta["text"].as_str() {
                                                        let _ = tx
                                                            .send(StreamChunk::TextDelta(
                                                                text.to_string(),
                                                            ))
                                                            .await;
                                                    }
                                                }
                                                Some("thinking_delta") => {
                                                    if let Some(text) = delta["thinking"].as_str() {
                                                        let _ = tx
                                                            .send(StreamChunk::Thinking(
                                                                text.to_string(),
                                                            ))
                                                            .await;
                                                    }
                                                }
                                                Some("input_json_delta") => {
                                                    if let Some(partial) =
                                                        delta["partial_json"].as_str()
                                                    {
                                                        current_tool_input.push_str(partial);
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        Some("content_block_stop") => {
                                            if in_tool_input {
                                                let arguments: serde_json::Value =
                                                    serde_json::from_str(&current_tool_input)
                                                        .unwrap_or_default();
                                                let _ = tx
                                                    .send(StreamChunk::ToolCall(
                                                        claw_core::ToolCall {
                                                            id: current_tool_id.clone(),
                                                            tool_name: current_tool_name.clone(),
                                                            arguments,
                                                        },
                                                    ))
                                                    .await;
                                                has_tool_calls = true;
                                                in_tool_input = false;
                                            }
                                        }
                                        Some("message_delta") => {
                                            if let Some(sr) = event["delta"]["stop_reason"].as_str()
                                            {
                                                stop_reason = match sr {
                                                    "tool_use" => StopReason::ToolUse,
                                                    "max_tokens" => StopReason::MaxTokens,
                                                    "stop_sequence" => StopReason::StopSequence,
                                                    _ => StopReason::EndTurn,
                                                };
                                            }
                                            if let Some(usage) = event["usage"].as_object() {
                                                if let Some(ot) = usage
                                                    .get("output_tokens")
                                                    .and_then(|v| v.as_u64())
                                                {
                                                    output_tokens = ot as u32;
                                                }
                                            }
                                        }
                                        Some("message_stop") => {
                                            let cost = estimate_anthropic_cost(
                                                &model,
                                                input_tokens,
                                                output_tokens,
                                            );
                                            let _ = tx
                                                .send(StreamChunk::Usage(Usage {
                                                    input_tokens,
                                                    output_tokens,
                                                    estimated_cost_usd: cost,
                                                    ..Default::default()
                                                }))
                                                .await;
                                            let final_stop = if has_tool_calls {
                                                StopReason::ToolUse
                                            } else {
                                                stop_reason
                                            };
                                            let _ = tx.send(StreamChunk::Done(final_stop)).await;
                                            return;
                                        }
                                        Some("error") => {
                                            let msg = event["error"]["message"]
                                                .as_str()
                                                .unwrap_or("unknown error");
                                            let _ =
                                                tx.send(StreamChunk::Error(msg.to_string())).await;
                                            return;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                                return;
                            }
                        }
                    }
                    // Stream ended without message_stop
                    let _ = tx.send(StreamChunk::Done(stop_reason)).await;
                }
                Ok(resp) => {
                    let text = resp.text().await.unwrap_or_default();
                    let _ = tx.send(StreamChunk::Error(text)).await;
                }
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                }
            }
        });

        Ok(rx)
    }

    async fn health_check(&self) -> Result<()> {
        info!("checking Anthropic API health");
        if self.api_key.is_empty() {
            return Err(claw_core::ClawError::LlmProvider(
                "ANTHROPIC_API_KEY not set".into(),
            ));
        }
        Ok(())
    }
}

/// Estimate cost for Anthropic models (USD per 1M tokens).
fn estimate_anthropic_cost(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_per_m, output_per_m) = match model {
        m if m.contains("opus") => (15.00, 75.00),
        m if m.contains("sonnet") => (3.00, 15.00),
        m if m.contains("haiku") => (0.80, 4.00),
        _ => (3.00, 15.00), // default to sonnet pricing
    };
    (input_tokens as f64 * input_per_m + output_tokens as f64 * output_per_m) / 1_000_000.0
}
