use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A goal the agent is pursuing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: Uuid,
    pub description: String,
    pub status: GoalStatus,
    pub priority: u8,
    pub progress: f32,
    pub steps: Vec<Step>,
    pub parent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// What the agent learned from this goal (filled on completion/failure).
    pub retrospective: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// A step in a goal's plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: Uuid,
    pub description: String,
    pub status: StepStatus,
    /// Tool calls needed for this step.
    pub tool_calls: Vec<String>,
    /// Result / output of the step.
    pub result: Option<String>,
    /// If the step failed, what went wrong.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    /// If this step was delegated to a mesh peer, the peer's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_to: Option<String>,
    /// The mesh task ID if this step was delegated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_task_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

/// The goal planner decomposes high-level goals into actionable steps
/// and manages the execution state machine.
pub struct GoalPlanner {
    /// Active goals, ordered by priority.
    goals: Vec<Goal>,
}

impl GoalPlanner {
    pub fn new() -> Self {
        Self { goals: Vec::new() }
    }

    /// Create a new top-level goal.
    pub fn create_goal(&mut self, description: String, priority: u8) -> &Goal {
        let goal = Goal {
            id: Uuid::new_v4(),
            description,
            status: GoalStatus::Active,
            priority,
            progress: 0.0,
            steps: Vec::new(),
            parent_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            retrospective: None,
        };
        self.goals.push(goal);
        self.goals.sort_by(|a, b| b.priority.cmp(&a.priority));
        self.goals.last().unwrap()
    }

    /// Create a sub-goal.
    pub fn create_subgoal(
        &mut self,
        parent_id: Uuid,
        description: String,
        priority: u8,
    ) -> Option<&Goal> {
        let goal = Goal {
            id: Uuid::new_v4(),
            description,
            status: GoalStatus::Active,
            priority,
            progress: 0.0,
            steps: Vec::new(),
            parent_id: Some(parent_id),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            retrospective: None,
        };
        self.goals.push(goal);
        self.goals.last()
    }

    /// Add steps to a goal (the plan).
    pub fn set_plan(&mut self, goal_id: Uuid, steps: Vec<String>) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            goal.steps = steps
                .into_iter()
                .map(|desc| Step {
                    id: Uuid::new_v4(),
                    description: desc,
                    status: StepStatus::Pending,
                    tool_calls: Vec::new(),
                    result: None,
                    error: None,
                    created_at: Utc::now(),
                    delegated_to: None,
                    delegated_task_id: None,
                })
                .collect();
            goal.updated_at = Utc::now();
        }
    }

    /// Get the next step to execute for a goal.
    pub fn next_step(&self, goal_id: Uuid) -> Option<&Step> {
        self.goals
            .iter()
            .find(|g| g.id == goal_id)
            .and_then(|g| g.steps.iter().find(|s| s.status == StepStatus::Pending))
    }

    /// Mark a step as in-progress.
    pub fn start_step(&mut self, goal_id: Uuid, step_id: Uuid) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            if let Some(step) = goal.steps.iter_mut().find(|s| s.id == step_id) {
                step.status = StepStatus::InProgress;
            }
            goal.updated_at = Utc::now();
        }
    }

    /// Complete a step with a result.
    pub fn complete_step(&mut self, goal_id: Uuid, step_id: Uuid, result: String) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            if let Some(step) = goal.steps.iter_mut().find(|s| s.id == step_id) {
                step.status = StepStatus::Completed;
                step.result = Some(result);
            }
            // Update progress
            let total = goal.steps.len() as f32;
            let done = goal
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count() as f32;
            goal.progress = if total > 0.0 { done / total } else { 0.0 };

            // Check if all steps are done
            if goal
                .steps
                .iter()
                .all(|s| s.status == StepStatus::Completed || s.status == StepStatus::Skipped)
            {
                goal.status = GoalStatus::Completed;
            }
            goal.updated_at = Utc::now();
        }
    }

    /// Fail a step, optionally failing the entire goal.
    pub fn fail_step(&mut self, goal_id: Uuid, step_id: Uuid, error: String, fail_goal: bool) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            if let Some(step) = goal.steps.iter_mut().find(|s| s.id == step_id) {
                step.status = StepStatus::Failed;
                step.error = Some(error.clone());
            }
            if fail_goal {
                goal.status = GoalStatus::Failed;
                goal.retrospective = Some(format!("Failed at step: {}", error));
            }
            goal.updated_at = Utc::now();
        }
    }

    /// Get all active goals.
    pub fn active_goals(&self) -> Vec<&Goal> {
        self.goals
            .iter()
            .filter(|g| g.status == GoalStatus::Active)
            .collect()
    }

    /// Get a specific goal.
    pub fn get(&self, goal_id: Uuid) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == goal_id)
    }

    /// Get all goals.
    pub fn all(&self) -> &[Goal] {
        &self.goals
    }

    /// Get all goals mutably.
    pub fn all_mut(&mut self) -> &mut Vec<Goal> {
        &mut self.goals
    }

    /// Restore a goal from persistent storage (SQLite).
    pub fn restore_goal(
        &mut self,
        id: Uuid,
        description: String,
        status: &str,
        priority: u8,
        progress: f32,
        parent_id: Option<Uuid>,
        steps: Vec<(Uuid, String, String, Option<String>)>,
    ) {
        let goal_status = match status {
            "active" => GoalStatus::Active,
            "paused" => GoalStatus::Paused,
            "completed" => GoalStatus::Completed,
            "failed" => GoalStatus::Failed,
            "cancelled" => GoalStatus::Cancelled,
            _ => GoalStatus::Active,
        };

        let restored_steps: Vec<Step> = steps
            .into_iter()
            .map(|(step_id, desc, step_status, result)| {
                let ss = match step_status.as_str() {
                    "pending" => StepStatus::Pending,
                    "in_progress" | "inprogress" => StepStatus::InProgress,
                    "completed" => StepStatus::Completed,
                    "failed" => StepStatus::Failed,
                    "skipped" => StepStatus::Skipped,
                    _ => StepStatus::Pending,
                };
                Step {
                    id: step_id,
                    description: desc,
                    status: ss,
                    tool_calls: Vec::new(),
                    result,
                    error: None,
                    created_at: Utc::now(),
                    delegated_to: None,
                    delegated_task_id: None,
                }
            })
            .collect();

        let goal = Goal {
            id,
            description,
            status: goal_status,
            priority,
            progress,
            steps: restored_steps,
            parent_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            retrospective: None,
        };

        self.goals.push(goal);
        self.goals.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Get the highest-priority active goal.
    pub fn current_goal(&self) -> Option<&Goal> {
        self.active_goals().into_iter().next()
    }

    /// Mark a step as delegated to a mesh peer.
    pub fn delegate_step(&mut self, goal_id: Uuid, step_id: Uuid, peer_id: String, task_id: Uuid) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            if let Some(step) = goal.steps.iter_mut().find(|s| s.id == step_id) {
                step.status = StepStatus::InProgress;
                step.delegated_to = Some(peer_id);
                step.delegated_task_id = Some(task_id);
            }
            goal.updated_at = Utc::now();
        }
    }

    /// Complete a step that was delegated via mesh, matched by task_id.
    /// Returns true if a matching step was found and completed.
    pub fn complete_delegated_task(&mut self, task_id: Uuid, result: String) -> bool {
        for goal in &mut self.goals {
            if let Some(step) = goal.steps.iter_mut().find(|s| {
                s.delegated_task_id == Some(task_id) && s.status == StepStatus::InProgress
            }) {
                step.status = StepStatus::Completed;
                step.result = Some(result);

                // Update goal progress
                let total = goal.steps.len() as f32;
                let done = goal
                    .steps
                    .iter()
                    .filter(|s| s.status == StepStatus::Completed)
                    .count() as f32;
                goal.progress = if total > 0.0 { done / total } else { 0.0 };

                // Check if all steps are done
                if goal
                    .steps
                    .iter()
                    .all(|s| s.status == StepStatus::Completed || s.status == StepStatus::Skipped)
                {
                    goal.status = GoalStatus::Completed;
                }
                goal.updated_at = Utc::now();
                return true;
            }
        }
        false
    }

    /// Fail a step that was delegated via mesh, matched by task_id.
    /// Returns true if a matching step was found and failed.
    pub fn fail_delegated_task(&mut self, task_id: Uuid, error: String) -> bool {
        for goal in &mut self.goals {
            if let Some(step) = goal.steps.iter_mut().find(|s| {
                s.delegated_task_id == Some(task_id) && s.status == StepStatus::InProgress
            }) {
                step.status = StepStatus::Failed;
                step.error = Some(error);
                goal.updated_at = Utc::now();
                return true;
            }
        }
        false
    }

    /// Find all steps currently delegated to mesh peers.
    pub fn delegated_steps(&self) -> Vec<(&Goal, &Step)> {
        let mut result = Vec::new();
        for goal in &self.goals {
            for step in &goal.steps {
                if step.delegated_to.is_some() && step.status == StepStatus::InProgress {
                    result.push((goal, step));
                }
            }
        }
        result
    }
}
