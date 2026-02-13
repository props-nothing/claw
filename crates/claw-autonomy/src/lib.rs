//! # claw-autonomy
//!
//! The autonomy and guardrail system. Implements five autonomy levels (L0-L4),
//! budget tracking, risk assessment, human-in-the-loop approval flows,
//! and a goal planning engine.

pub mod approval;
pub mod budget;
pub mod guardrail;
pub mod level;
pub mod planner;

pub use approval::{ApprovalGate, ApprovalRequest, ApprovalResponse};
pub use budget::BudgetTracker;
pub use guardrail::{Guardrail, GuardrailEngine, GuardrailVerdict};
pub use level::AutonomyLevel;
pub use planner::{Goal, GoalPlanner, GoalStatus, Step};
