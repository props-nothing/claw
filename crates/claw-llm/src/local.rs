use async_trait::async_trait;
use claw_core::Result;
use tracing::info;

use crate::provider::*;

/// Local model provider — wraps llama.cpp / MLX / Ollama / any OpenAI-compatible local server.
pub struct LocalProvider {
    client: reqwest::Client,
    /// Address of the local inference server (e.g. "http://127.0.0.1:11434")
    base_url: String,
    model_name: String,
}

impl LocalProvider {
    pub fn new(base_url: String, model_name: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model_name,
        }
    }

    /// Default Ollama instance
    pub fn ollama(model: &str) -> Self {
        Self::new("http://127.0.0.1:11434".into(), model.to_string())
    }
}

#[async_trait]
impl LlmProvider for LocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn models(&self) -> Vec<String> {
        vec![self.model_name.clone()]
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        // Use Ollama-compatible API (OpenAI format at /v1/chat/completions)
        let mut messages = Vec::new();

        if let Some(ref system) = request.system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }

        for msg in &request.messages {
            let role = match msg.role {
                claw_core::Role::System => "system",
                claw_core::Role::User => "user",
                claw_core::Role::Assistant => "assistant",
                claw_core::Role::Tool => "tool",
            };
            messages.push(serde_json::json!({
                "role": role,
                "content": msg.text_content(),
            }));
        }

        let body = serde_json::json!({
            "model": &request.model,
            "messages": messages,
            "stream": false,
            "options": {
                "temperature": request.temperature,
                "num_predict": request.max_tokens,
            }
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(format!("local: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::LlmProvider(format!(
                "local model error: {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

        let content = data["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let message =
            claw_core::Message::text(uuid::Uuid::nil(), claw_core::Role::Assistant, content);

        Ok(LlmResponse {
            message,
            usage: Usage {
                input_tokens: data["prompt_eval_count"].as_u64().unwrap_or(0) as u32,
                output_tokens: data["eval_count"].as_u64().unwrap_or(0) as u32,
                estimated_cost_usd: 0.0, // Local = free
                ..Default::default()
            },
            has_tool_calls: false,
            stop_reason: StopReason::EndTurn,
        })
    }

    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let mut messages = Vec::new();
        if let Some(ref system) = request.system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }
        for msg in &request.messages {
            let role = match msg.role {
                claw_core::Role::System => "system",
                claw_core::Role::User => "user",
                claw_core::Role::Assistant => "assistant",
                claw_core::Role::Tool => "tool",
            };
            messages.push(serde_json::json!({
                "role": role,
                "content": msg.text_content(),
            }));
        }

        let body = serde_json::json!({
            "model": &request.model,
            "messages": messages,
            "stream": true,
            "options": {
                "temperature": request.temperature,
                "num_predict": request.max_tokens,
            }
        });

        let client = self.client.clone();
        let base_url = self.base_url.clone();

        tokio::spawn(async move {
            let resp = client
                .post(format!("{base_url}/api/chat"))
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(resp) if resp.status().is_success() => {
                    use futures::StreamExt;
                    let mut stream = resp.bytes_stream();
                    let mut buffer = String::new();
                    let mut input_tokens = 0u32;
                    let mut output_tokens = 0u32;

                    while let Some(chunk_result) = stream.next().await {
                        match chunk_result {
                            Ok(bytes) => {
                                buffer.push_str(&String::from_utf8_lossy(&bytes));
                                // Ollama sends newline-delimited JSON
                                while let Some(newline_pos) = buffer.find('\n') {
                                    let line = buffer[..newline_pos].trim().to_string();
                                    buffer = buffer[newline_pos + 1..].to_string();
                                    if line.is_empty() {
                                        continue;
                                    }
                                    if let Ok(event) =
                                        serde_json::from_str::<serde_json::Value>(&line)
                                    {
                                        // Content delta
                                        if let Some(content) = event["message"]["content"].as_str()
                                            && !content.is_empty()
                                        {
                                            let _ = tx
                                                .send(StreamChunk::TextDelta(content.to_string()))
                                                .await;
                                        }
                                        // Final message has "done": true
                                        if event["done"].as_bool() == Some(true) {
                                            if let Some(pt) = event["prompt_eval_count"].as_u64() {
                                                input_tokens = pt as u32;
                                            }
                                            if let Some(et) = event["eval_count"].as_u64() {
                                                output_tokens = et as u32;
                                            }
                                            let _ = tx
                                                .send(StreamChunk::Usage(Usage {
                                                    input_tokens,
                                                    output_tokens,
                                                    estimated_cost_usd: 0.0,
                                                    ..Default::default()
                                                }))
                                                .await;
                                            let _ = tx
                                                .send(StreamChunk::Done(StopReason::EndTurn))
                                                .await;
                                            return;
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
                    let _ = tx.send(StreamChunk::Done(StopReason::EndTurn)).await;
                }
                Ok(resp) => {
                    let text = resp.text().await.unwrap_or_default();
                    let _ = tx.send(StreamChunk::Error(text)).await;
                }
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(format!("local: {e}"))).await;
                }
            }
        });

        Ok(rx)
    }

    async fn health_check(&self) -> Result<()> {
        info!(base_url = %self.base_url, "checking local model health");
        let resp = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(format!("local unreachable: {e}")))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(claw_core::ClawError::LlmProvider(
                "local model server unhealthy".into(),
            ))
        }
    }
}
