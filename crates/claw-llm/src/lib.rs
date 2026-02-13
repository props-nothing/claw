//! # claw-llm
//!
//! Abstraction layer over LLM providers. Supports streaming, tool use,
//! thinking/reasoning, and automatic failover between providers.

pub mod provider;
pub mod router;
pub mod anthropic;
pub mod openai;
pub mod local;
pub mod embedding;
pub mod mock;

pub use provider::{LlmProvider, LlmRequest, LlmResponse, StopReason, StreamChunk, Usage};
pub use router::ModelRouter;
pub use embedding::EmbeddingProvider;
pub use mock::MockProvider;
