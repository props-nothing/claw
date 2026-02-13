//! # claw-memory
//!
//! Three-tier memory system for the Claw agent:
//!
//! - **Working memory**: Current conversation context (in-memory, ephemeral).
//! - **Episodic memory**: Past conversations, events, outcomes (SQLite, persistent).
//! - **Semantic memory**: Facts, knowledge, embeddings (SQLite + vector index, persistent).
//!
//! The memory system enables the agent to learn from past interactions,
//! recall relevant context, and build long-term knowledge.

pub mod episodic;
pub mod semantic;
pub mod store;
pub mod working;

pub use episodic::{Episode, EpisodicMemory};
pub use semantic::{Fact, SemanticMemory};
pub use store::MemoryStore;
pub use store::{GoalRow, GoalStepRow, SessionRow};
pub use working::WorkingMemory;
