//! # claw-autonomy
//!
//! The autonomy and guardrail system. Implements five autonomy levels (L0-L4),
//! budget tracking, risk assessment, human-in-the-loop approval flows,
//! and a goal planning engine.

pub mod level;
pub mod guardrail;
pub mod budget;
pub mod planner;
pub mod approval;

pub use level::AutonomyLevel;
pub use guardrail::{Guardrail, GuardrailEngine, GuardrailVerdict};
pub use budget::BudgetTracker;
pub use planner::{Goal, GoalPlanner, GoalStatus, Step};
pub use approval::{ApprovalRequest, ApprovalResponse, ApprovalGate};
