use serde::{Deserialize, Serialize};
use std::fmt;

/// Five autonomy levels, inspired by autonomous driving:
///
/// - **L0 (Manual)**: Every tool call requires explicit human approval.
/// - **L1 (Assisted)**: Auto-handles routine/read-only actions, asks for anything novel.
/// - **L2 (Supervised)**: Acts freely on most tasks, sends periodic summaries.
/// - **L3 (Autonomous)**: Pursues goals independently, only escalates high-risk actions.
/// - **L4 (Full Auto)**: Fully self-directed within budget/scope constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AutonomyLevel {
    Manual = 0,
    Assisted = 1,
    Supervised = 2,
    Autonomous = 3,
    FullAuto = 4,
}

impl AutonomyLevel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Manual,
            1 => Self::Assisted,
            2 => Self::Supervised,
            3 => Self::Autonomous,
            4 => Self::FullAuto,
            _ => Self::Assisted, // safe default
        }
    }

    /// Whether this level allows any autonomous action.
    pub fn allows_autonomous_action(&self) -> bool {
        *self >= Self::Assisted
    }

    /// Whether this level supports proactive goal pursuit.
    pub fn allows_proactive_goals(&self) -> bool {
        *self >= Self::Autonomous
    }

    /// Whether this level auto-approves tool calls below a risk threshold.
    pub fn auto_approve_threshold(&self) -> u8 {
        match self {
            Self::Manual => 0,     // nothing auto-approved
            Self::Assisted => 3,   // read-only, low-risk
            Self::Supervised => 5, // moderate actions
            Self::Autonomous => 7, // most actions
            Self::FullAuto => 9,   // nearly everything
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Manual => "Every action requires approval",
            Self::Assisted => "Routine actions auto-approved, novel actions need approval",
            Self::Supervised => "Acts freely, sends periodic summaries",
            Self::Autonomous => "Pursues goals independently, escalates high-risk only",
            Self::FullAuto => "Fully self-directed within budget/scope constraints",
        }
    }
}

impl fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{} ({})", *self as u8, match self {
            Self::Manual => "Manual",
            Self::Assisted => "Assisted",
            Self::Supervised => "Supervised",
            Self::Autonomous => "Autonomous",
            Self::FullAuto => "Full Auto",
        })
    }
}
