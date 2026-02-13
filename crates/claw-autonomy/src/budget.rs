use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;

/// Tracks spending budgets (LLM API costs, tool call counts, etc.)
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    state: Arc<RwLock<BudgetState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetState {
    /// Current day (resets daily).
    pub current_day: String,
    /// USD spent today.
    pub daily_spend_usd: f64,
    /// Daily budget limit in USD.
    pub daily_limit_usd: f64,
    /// Tool calls made in the current agent loop.
    pub loop_tool_calls: u32,
    /// Max tool calls per loop.
    pub max_tool_calls_per_loop: u32,
    /// Total spend since tracking started.
    pub total_spend_usd: f64,
    /// Total tool calls since tracking started.
    pub total_tool_calls: u64,
}

impl BudgetTracker {
    pub fn new(daily_limit_usd: f64, max_tool_calls_per_loop: u32) -> Self {
        Self {
            state: Arc::new(RwLock::new(BudgetState {
                current_day: today(),
                daily_spend_usd: 0.0,
                daily_limit_usd,
                loop_tool_calls: 0,
                max_tool_calls_per_loop,
                total_spend_usd: 0.0,
                total_tool_calls: 0,
            })),
        }
    }

    /// Record LLM spending.
    pub fn record_spend(&self, usd: f64) -> claw_core::Result<()> {
        let mut state = self.state.write();
        self.maybe_reset_day(&mut state);

        state.daily_spend_usd += usd;
        state.total_spend_usd += usd;

        if state.daily_spend_usd > state.daily_limit_usd {
            warn!(
                spent = state.daily_spend_usd,
                limit = state.daily_limit_usd,
                "daily budget exceeded"
            );
            return Err(claw_core::ClawError::BudgetExceeded {
                resource: "daily_spend_usd".into(),
                used: state.daily_spend_usd,
                limit: state.daily_limit_usd,
            });
        }
        Ok(())
    }

    /// Record a tool call in the current loop.
    pub fn record_tool_call(&self) -> claw_core::Result<()> {
        let mut state = self.state.write();
        state.loop_tool_calls += 1;
        state.total_tool_calls += 1;

        if state.loop_tool_calls > state.max_tool_calls_per_loop {
            return Err(claw_core::ClawError::BudgetExceeded {
                resource: "tool_calls_per_loop".into(),
                used: state.loop_tool_calls as f64,
                limit: state.max_tool_calls_per_loop as f64,
            });
        }
        Ok(())
    }

    /// Reset the per-loop tool call counter (called at the start of each agent loop).
    pub fn reset_loop(&self) {
        self.state.write().loop_tool_calls = 0;
    }

    /// Check if we're within budget without recording.
    pub fn check(&self) -> claw_core::Result<()> {
        let state = self.state.read();
        if state.daily_spend_usd >= state.daily_limit_usd {
            return Err(claw_core::ClawError::BudgetExceeded {
                resource: "daily_spend_usd".into(),
                used: state.daily_spend_usd,
                limit: state.daily_limit_usd,
            });
        }
        Ok(())
    }

    /// Get the current budget state.
    pub fn snapshot(&self) -> BudgetState {
        self.state.read().clone()
    }

    fn maybe_reset_day(&self, state: &mut BudgetState) {
        let day = today();
        if state.current_day != day {
            state.current_day = day;
            state.daily_spend_usd = 0.0;
        }
    }
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}
