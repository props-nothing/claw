//! # claw-core
//!
//! Core types, traits, and primitives for the Claw autonomous AI agent runtime.
//! This crate defines the shared vocabulary used by every other crate in the workspace.

pub mod error;
pub mod event;
pub mod message;
pub mod tool;
pub mod types;

pub use error::{ClawError, Result};
pub use event::{Event, EventBus};
pub use message::{Message, MessageContent, Role};
pub use tool::{Tool, ToolCall, ToolResult, ToolExecutor};
pub use types::*;
