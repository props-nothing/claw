use claw_core::{Tool, ToolCall};
use tracing::{info, warn};

use crate::level::AutonomyLevel;

/// A guardrail rule that can approve, deny, or escalate a tool call.
#[derive(Debug, Clone)]
pub enum GuardrailVerdict {
    /// Action is approved to proceed.
    Approve,
    /// Action is denied — provide reason.
    Deny(String),
    /// Action needs human approval — provide reason.
    Escalate(String),
}

/// A single guardrail rule.
pub trait Guardrail: Send + Sync {
    fn name(&self) -> &str;
    fn evaluate(&self, tool: &Tool, call: &ToolCall, level: AutonomyLevel) -> GuardrailVerdict;
}

/// The guardrail engine applies all registered rules to a tool call.
pub struct GuardrailEngine {
    rules: Vec<Box<dyn Guardrail>>,
    allowlist: Vec<String>,
    denylist: Vec<String>,
}

impl Default for GuardrailEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GuardrailEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            rules: Vec::new(),
            allowlist: Vec::new(),
            denylist: Vec::new(),
        };
        // Register built-in guardrails
        engine.add_rule(Box::new(RiskLevelGuardrail));
        engine.add_rule(Box::new(DestructiveActionGuardrail { max_deletes: 5 }));
        engine.add_rule(Box::new(NetworkExfiltrationGuardrail));
        engine
    }

    pub fn add_rule(&mut self, rule: Box<dyn Guardrail>) {
        self.rules.push(rule);
    }

    pub fn set_allowlist(&mut self, list: Vec<String>) {
        self.allowlist = list;
    }

    pub fn set_denylist(&mut self, list: Vec<String>) {
        self.denylist = list;
    }

    /// Evaluate a tool call against all guardrails.
    pub fn evaluate(&self, tool: &Tool, call: &ToolCall, level: AutonomyLevel) -> GuardrailVerdict {
        // Check denylist first
        if self.denylist.iter().any(|d| d == &tool.name) {
            warn!(tool = %tool.name, "tool is on denylist");
            return GuardrailVerdict::Deny(format!("tool '{}' is on the denylist", tool.name));
        }

        // Check allowlist — always approve
        if self.allowlist.iter().any(|a| a == &tool.name) {
            return GuardrailVerdict::Approve;
        }

        // Run all guardrail rules
        for rule in &self.rules {
            match rule.evaluate(tool, call, level) {
                GuardrailVerdict::Approve => continue,
                verdict @ GuardrailVerdict::Deny(_) => {
                    info!(
                        rule = rule.name(),
                        tool = %tool.name,
                        "guardrail denied action"
                    );
                    return verdict;
                }
                verdict @ GuardrailVerdict::Escalate(_) => {
                    info!(
                        rule = rule.name(),
                        tool = %tool.name,
                        "guardrail escalated action for approval"
                    );
                    return verdict;
                }
            }
        }

        GuardrailVerdict::Approve
    }
}

// ── Built-in guardrails ────────────────────────────────────────

/// Checks tool risk_level against the autonomy level's auto-approve threshold.
struct RiskLevelGuardrail;

impl Guardrail for RiskLevelGuardrail {
    fn name(&self) -> &str {
        "risk_level"
    }

    fn evaluate(&self, tool: &Tool, _call: &ToolCall, level: AutonomyLevel) -> GuardrailVerdict {
        let threshold = level.auto_approve_threshold();
        if tool.risk_level > threshold {
            GuardrailVerdict::Escalate(format!(
                "tool '{}' has risk level {} which exceeds threshold {} for {}",
                tool.name, tool.risk_level, threshold, level
            ))
        } else {
            GuardrailVerdict::Approve
        }
    }
}

/// Prevents mass file deletion.
struct DestructiveActionGuardrail {
    max_deletes: u32,
}

impl Guardrail for DestructiveActionGuardrail {
    fn name(&self) -> &str {
        "destructive_action"
    }

    fn evaluate(&self, tool: &Tool, call: &ToolCall, _level: AutonomyLevel) -> GuardrailVerdict {
        // Check if this is a delete/remove operation
        if tool.name.contains("delete") || tool.name.contains("remove") || tool.name.contains("rm")
        {
            // Check if it's operating on multiple targets
            if let Some(paths) = call.arguments.get("paths") {
                if let Some(arr) = paths.as_array() {
                    if arr.len() > self.max_deletes as usize {
                        return GuardrailVerdict::Deny(format!(
                            "attempting to delete {} files, max allowed is {}",
                            arr.len(),
                            self.max_deletes
                        ));
                    }
                }
            }
            // Single deletes in lower autonomy levels need approval
            if _level < AutonomyLevel::Supervised {
                return GuardrailVerdict::Escalate("delete operation requires approval".into());
            }
        }
        GuardrailVerdict::Approve
    }
}

/// Detects potential data exfiltration via network requests.
struct NetworkExfiltrationGuardrail;

impl Guardrail for NetworkExfiltrationGuardrail {
    fn name(&self) -> &str {
        "network_exfiltration"
    }

    fn evaluate(&self, tool: &Tool, call: &ToolCall, _level: AutonomyLevel) -> GuardrailVerdict {
        // If a shell command is piping file content to curl/wget, that's suspicious
        if tool.name == "shell_exec" || tool.name == "system_run" {
            if let Some(cmd) = call.arguments.get("command").and_then(|v| v.as_str()) {
                let suspicious = cmd.contains("curl")
                    && (cmd.contains("cat ") || cmd.contains("< /"))
                    || cmd.contains("wget") && cmd.contains("--post-file");
                if suspicious {
                    return GuardrailVerdict::Escalate(
                        "command may be exfiltrating data via network".into(),
                    );
                }
            }
        }
        GuardrailVerdict::Approve
    }
}
