//! # claw-llm
//!
//! Abstraction layer over LLM providers. Supports streaming, tool use,
//! thinking/reasoning, and automatic failover between providers.

pub mod anthropic;
pub mod embedding;
pub mod local;
pub mod mock;
pub mod openai;
pub mod provider;
pub mod router;

pub use embedding::EmbeddingProvider;
pub use mock::MockProvider;
pub use provider::{LlmProvider, LlmRequest, LlmResponse, StopReason, StreamChunk, Usage};
pub use router::ModelRouter;
