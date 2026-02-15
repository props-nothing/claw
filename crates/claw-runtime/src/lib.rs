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
pub(crate) mod agent_loop;
pub(crate) mod channel_helpers;
pub(crate) mod learning;
pub(crate) mod query;
pub mod scheduler;
pub mod session;
pub(crate) mod sub_agent;
pub mod terminal;
pub(crate) mod tool_dispatch;
pub mod tools;

pub use agent::AgentRuntime;
pub use agent::{
    ApiResponse, Notification, RuntimeHandle, StreamEvent, get_runtime_handle, set_runtime_handle,
};
pub use agent::{PendingSubTasks, SubTaskState, SubTaskStatus};
pub use agent::{SharedAgentState, build_test_state, build_test_state_with_router};
pub use query::QueryKind;
pub use scheduler::{CronScheduler, ScheduledTask, SchedulerHandle};
pub use session::Session;
