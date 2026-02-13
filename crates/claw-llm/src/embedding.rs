use async_trait::async_trait;
use claw_core::Result;
use tracing::debug;

/// Trait for generating text embeddings.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embeddings for a batch of texts.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// The dimensionality of the output embeddings.
    fn dimensions(&self) -> usize;

    /// Provider name.
    fn name(&self) -> &str;
}

/// OpenAI embeddings provider (text-embedding-3-small, text-embedding-3-large, etc.)
pub struct OpenAiEmbedding {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    dims: usize,
}

impl OpenAiEmbedding {
    /// Create an OpenAI embedding provider with text-embedding-3-small (1536 dims).
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".into(),
            model: "text-embedding-3-small".into(),
            dims: 1536,
        }
    }

    /// Use a specific model (e.g. "text-embedding-3-large" with 3072 dims).
    pub fn with_model(mut self, model: String, dims: usize) -> Self {
        self.model = model;
        self.dims = dims;
        self
    }

    /// Use a custom base URL (e.g. for Azure OpenAI).
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedding {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        debug!(model = %self.model, count = texts.len(), "generating embeddings");

        let body = serde_json::json!({
            "model": &self.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(format!("embedding request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::LlmProvider(format!(
                "embedding HTTP {}: {}",
                status, text
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| claw_core::ClawError::LlmProvider(format!("embedding parse error: {}", e)))?;

        let embeddings: Vec<Vec<f32>> = data["data"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item["embedding"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect()
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &str {
        "openai"
    }
}

/// Ollama embeddings provider (uses /api/embeddings endpoint).
pub struct OllamaEmbedding {
    client: reqwest::Client,
    base_url: String,
    model: String,
    dims: usize,
}

impl OllamaEmbedding {
    pub fn new(model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "http://127.0.0.1:11434".into(),
            model: model.to_string(),
            dims: 768, // common default, varies by model
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let body = serde_json::json!({
                "model": &self.model,
                "prompt": text,
            });

            let resp = self
                .client
                .post(format!("{}/api/embeddings", self.base_url))
                .json(&body)
                .send()
                .await
                .map_err(|e| claw_core::ClawError::LlmProvider(format!("ollama embedding: {}", e)))?;

            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(claw_core::ClawError::LlmProvider(format!(
                    "ollama embedding error: {}",
                    text
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| claw_core::ClawError::LlmProvider(e.to_string()))?;

            let embedding: Vec<f32> = data["embedding"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default();

            if !embedding.is_empty() {
                results.push(embedding);
            }
        }

        Ok(results)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &str {
        "ollama"
    }
}
