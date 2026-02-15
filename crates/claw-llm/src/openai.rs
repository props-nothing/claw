use async_trait::async_trait;
use claw_core::Result;
use tracing::info;

use crate::provider::*;

/// OpenAI-compatible API provider (works with OpenAI, Azure, Together, etc.)
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    provider_name: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".into(),
            provider_name: "openai".into(),
        }
    }

    /// Use a custom base URL (for Azure, Together, vLLM, etc.)
    pub fn with_base_url(mut self, url: String, name: String) -> Self {
        self.base_url = url;
        self.provider_name = name;
        self
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn models(&self) -> Vec<String> {
        vec![
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "o1".into(),
            "o1-mini".into(),
            "o3".into(),
            "o3-mini".into(),
        ]
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let mut messages = Vec::new();

        if let Some(ref system) = request.system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }

        for msg in &request.messages {
            match msg.role {
                claw_core::Role::System => {
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content": msg.text_content(),
                    }));
                }
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
                        // Assistant message with tool calls — must include tool_calls array
                        let tc: Vec<serde_json::Value> = msg.tool_calls.iter().map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.tool_name,
                                    "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default(),
                                }
                            })
                        }).collect();
                        let text = msg.text_content();
                        let content = if text.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(text)
                        };
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": tc,
                        }));
                    }
                }
                claw_core::Role::Tool => {
                    // Tool result messages — extract tool_call_id from ToolResult content blocks
                    for block in &msg.content {
                        if let claw_core::MessageContent::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } = block
                        {
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_call_id,
                                "content": content,
                            }));
                        }
                    }
                    // Fallback: if no ToolResult blocks, send as user message to avoid API errors
                    if !msg
                        .content
                        .iter()
                        .any(|c| matches!(c, claw_core::MessageContent::ToolResult { .. }))
                    {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": msg.text_content(),
                        }));
                    }
                }
            }
        }

        let mut body = serde_json::json!({
            "model": &request.model,
            "temperature": request.temperature,
            "messages": messages,
        });

        // Newer OpenAI models (o1, o3, gpt-5, …) require max_completion_tokens
        if uses_max_completion_tokens(&request.model) {
            body["max_completion_tokens"] = serde_json::json!(request.max_tokens);
        } else {
            body["max_tokens"] = serde_json::json!(request.max_tokens);
        }

        if !request.tools.is_empty() {
            let tools: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::LlmProvider(format!(
                "HTTP {status}: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

        let choice = &data["choices"][0];
        let content = choice["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let tool_calls: Vec<claw_core::ToolCall> = choice["message"]["tool_calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| {
                        Some(claw_core::ToolCall {
                            id: c["id"].as_str()?.to_string(),
                            tool_name: c["function"]["name"].as_str()?.to_string(),
                            arguments: serde_json::from_str(
                                c["function"]["arguments"].as_str().unwrap_or("{}"),
                            )
                            .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let has_tool_calls = !tool_calls.is_empty();

        let mut message =
            claw_core::Message::text(uuid::Uuid::nil(), claw_core::Role::Assistant, content);
        message.tool_calls = tool_calls;

        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        let usage_data = &data["usage"];
        let input_tokens = usage_data["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage_data["completion_tokens"].as_u64().unwrap_or(0) as u32;
        let estimated_cost_usd = estimate_openai_cost(&request.model, input_tokens, output_tokens);

        Ok(LlmResponse {
            message,
            usage: Usage {
                input_tokens,
                output_tokens,
                thinking_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                estimated_cost_usd,
            },
            has_tool_calls,
            stop_reason: match finish_reason {
                "length" => StopReason::MaxTokens,
                "content_filter" => StopReason::ContentFilter,
                _ if has_tool_calls => StopReason::ToolUse,
                _ => StopReason::EndTurn,
            },
        })
    }

    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        // Build the same body as complete() but add stream: true
        let mut messages = Vec::new();

        if let Some(ref system) = request.system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }

        for msg in &request.messages {
            match msg.role {
                claw_core::Role::System => {
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content": msg.text_content(),
                    }));
                }
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
                        let tc: Vec<serde_json::Value> = msg.tool_calls.iter().map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.tool_name,
                                    "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default(),
                                }
                            })
                        }).collect();
                        let text = msg.text_content();
                        let content = if text.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(text)
                        };
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": tc,
                        }));
                    }
                }
                claw_core::Role::Tool => {
                    for block in &msg.content {
                        if let claw_core::MessageContent::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } = block
                        {
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_call_id,
                                "content": content,
                            }));
                        }
                    }
                    if !msg
                        .content
                        .iter()
                        .any(|c| matches!(c, claw_core::MessageContent::ToolResult { .. }))
                    {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": msg.text_content(),
                        }));
                    }
                }
            }
        }

        let mut body = serde_json::json!({
            "model": &request.model,
            "temperature": request.temperature,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        // Newer OpenAI models (o1, o3, gpt-5, …) require max_completion_tokens
        if uses_max_completion_tokens(&request.model) {
            body["max_completion_tokens"] = serde_json::json!(request.max_tokens);
        } else {
            body["max_tokens"] = serde_json::json!(request.max_tokens);
        }

        if !request.tools.is_empty() {
            let tools: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let model = request.model.clone();

        tokio::spawn(async move {
            let resp = client
                .post(format!("{base_url}/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(resp) if resp.status().is_success() => {
                    use futures::StreamExt;
                    let mut stream = resp.bytes_stream();
                    let mut buffer = String::new();
                    // Track tool call deltas: index -> (id, name, arguments_json)
                    let mut tool_calls: std::collections::HashMap<u64, (String, String, String)> =
                        std::collections::HashMap::new();
                    let mut input_tokens = 0u32;
                    let mut output_tokens = 0u32;
                    let mut finish_reason_str: Option<String> = None;

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
                                    if let Some(data) = line.strip_prefix("data: ") {
                                        if data.trim() == "[DONE]" {
                                            // Emit any accumulated tool calls
                                            for (id, name, args) in tool_calls.values() {
                                                let arguments: serde_json::Value =
                                                    serde_json::from_str(args).unwrap_or_default();
                                                let _ = tx
                                                    .send(StreamChunk::ToolCall(
                                                        claw_core::ToolCall {
                                                            id: id.clone(),
                                                            tool_name: name.clone(),
                                                            arguments,
                                                        },
                                                    ))
                                                    .await;
                                            }
                                            let stop = match finish_reason_str.as_deref() {
                                                Some("length") => StopReason::MaxTokens,
                                                Some("content_filter") => StopReason::ContentFilter,
                                                _ if !tool_calls.is_empty() => StopReason::ToolUse,
                                                _ => StopReason::EndTurn,
                                            };
                                            let cost = estimate_openai_cost(
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
                                            let _ = tx.send(StreamChunk::Done(stop)).await;
                                            return;
                                        }
                                        if let Ok(event) =
                                            serde_json::from_str::<serde_json::Value>(data)
                                        {
                                            let delta = &event["choices"][0]["delta"];
                                            // Text content
                                            if let Some(text) = delta["content"].as_str()
                                                && !text.is_empty() {
                                                    let _ = tx
                                                        .send(StreamChunk::TextDelta(
                                                            text.to_string(),
                                                        ))
                                                        .await;
                                                }
                                            // Tool call deltas
                                            if let Some(tcs) = delta["tool_calls"].as_array() {
                                                for tc in tcs {
                                                    let idx = tc["index"].as_u64().unwrap_or(0);
                                                    let entry = tool_calls
                                                        .entry(idx)
                                                        .or_insert_with(|| {
                                                            (
                                                                String::new(),
                                                                String::new(),
                                                                String::new(),
                                                            )
                                                        });
                                                    if let Some(id) = tc["id"].as_str() {
                                                        entry.0 = id.to_string();
                                                    }
                                                    if let Some(name) =
                                                        tc["function"]["name"].as_str()
                                                    {
                                                        entry.1.push_str(name);
                                                    }
                                                    if let Some(args) =
                                                        tc["function"]["arguments"].as_str()
                                                    {
                                                        entry.2.push_str(args);
                                                    }
                                                }
                                            }
                                            // Track finish_reason from the choice
                                            if let Some(fr) =
                                                event["choices"][0]["finish_reason"].as_str()
                                            {
                                                finish_reason_str = Some(fr.to_string());
                                            }
                                            // Usage (in final chunk with stream_options)
                                            if let Some(usage) = event.get("usage") {
                                                if let Some(pt) = usage["prompt_tokens"].as_u64() {
                                                    input_tokens = pt as u32;
                                                }
                                                if let Some(ct) =
                                                    usage["completion_tokens"].as_u64()
                                                {
                                                    output_tokens = ct as u32;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                                return;
                            }
                        }
                    }
                    // Stream ended without [DONE]
                    let _ = tx.send(StreamChunk::Done(StopReason::EndTurn)).await;
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
        info!(provider = self.provider_name, "checking API health");
        if self.api_key.is_empty() {
            return Err(claw_core::ClawError::LlmProvider(format!(
                "{} API key not set",
                self.provider_name
            )));
        }
        Ok(())
    }
}

/// Returns true for models that require `max_completion_tokens` instead of `max_tokens`.
fn uses_max_completion_tokens(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("gpt-5")
        || m.contains("gpt5")
}

/// Estimate cost for OpenAI models (USD per 1M tokens).
fn estimate_openai_cost(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_per_m, output_per_m) = match model {
        m if m.starts_with("gpt-4o-mini") => (0.15, 0.60),
        m if m.starts_with("gpt-4o") => (2.50, 10.00),
        m if m.starts_with("gpt-4-turbo") => (10.00, 30.00),
        m if m.starts_with("gpt-4") => (30.00, 60.00),
        m if m.contains("gpt-5") || m.contains("gpt5") => (2.50, 10.00),
        m if m.starts_with("o3-mini") => (1.10, 4.40),
        m if m.starts_with("o3") => (10.00, 40.00),
        m if m.starts_with("o4-mini") => (1.10, 4.40),
        m if m.starts_with("o1-mini") => (3.00, 12.00),
        m if m.starts_with("o1") => (15.00, 60.00),
        _ => (2.50, 10.00), // default to gpt-4o pricing
    };
    (input_tokens as f64 * input_per_m + output_tokens as f64 * output_per_m) / 1_000_000.0
}
