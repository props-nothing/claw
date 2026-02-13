//! # claw-runtime
//!
//! The agent runtime — the central loop that orchestrates the LLM, tools,
//! memory, autonomy system, plugins, channels, and mesh network.
//!
//! ## Architecture
//!
//! ```text
//!              ┌─────────────┐
//!              │   Channels   │  ← Telegram, Discord, WebChat, ...
//!              └──────┬───────┘
//!                     │ IncomingMessage
//!                     ▼
//!              ┌──────────────┐
//!              │  Agent Loop  │  ← The core reasoning cycle
//!              │              │
//!              │  1. Receive  │
//!              │  2. Recall   │  ← Memory retrieval
//!              │  3. Think    │  ← LLM call (with tools)
//!              │  4. Guard    │  ← Autonomy/guardrail check
//!              │  5. Act      │  ← Tool execution
//!              │  6. Reflect  │  ← Outcome evaluation
//!              │  7. Remember │  ← Memory storage
//!              │  8. Respond  │  ← Send reply
//!              └──────────────┘
//!                     │
//!         ┌───────────┼───────────┐
//!         ▼           ▼           ▼
//!    ┌─────────┐ ┌─────────┐ ┌────────┐
//!    │  LLM    │ │ Memory  │ │ Plugins│
//!    │ Router  │ │  Store  │ │  Host  │
//!    └─────────┘ └─────────┘ └────────┘
//! ```

pub mod agent;
pub mod session;
pub mod terminal;
pub mod tools;

pub use agent::AgentRuntime;
pub use agent::{
    ApiResponse, QueryKind, RuntimeHandle, StreamEvent, get_runtime_handle, set_runtime_handle,
};
pub use agent::{SharedAgentState, build_test_state, build_test_state_with_router};
pub use session::Session;
