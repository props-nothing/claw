//! # Cron & One-Shot Scheduler
//!
//! Provides recurring cron-based scheduling and one-shot delayed execution.
//! Scheduled tasks are injected as synthetic messages into the agent runtime
//! so they execute through the same pipeline as user messages.
//!
//! Two types of scheduled tasks:
//! - **Recurring**: Fires on a cron expression (e.g., `"*/5 * * * *"` for every 5 minutes).
//! - **OneShot**: Fires once after a delay (e.g., 60 seconds from now).
//!
//! Persisted to SQLite so scheduled tasks survive restarts.

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// A scheduled task — either recurring (cron) or one-shot (delay).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: Uuid,
    /// Human-readable label for this task.
    pub label: Option<String>,
    /// The prompt/description that will be sent to the agent when the task fires.
    pub description: String,
    /// Kind of schedule.
    pub kind: ScheduleKind,
    /// When the task was created.
    pub created_at: DateTime<Utc>,
    /// Session ID to use when firing (None = create new session).
    pub session_id: Option<Uuid>,
    /// Whether this task is active.
    pub active: bool,
    /// Number of times this task has fired.
    pub fire_count: u64,
    /// When this task last fired.
    pub last_fired: Option<DateTime<Utc>>,
}

/// The kind of schedule: recurring cron or one-shot delay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleKind {
    /// Recurring schedule based on a cron expression.
    Cron { expression: String },
    /// One-shot: fires once at the specified time.
    OneShot { fire_at: DateTime<Utc> },
}

/// A message emitted by the scheduler when a task should fire.
#[derive(Debug, Clone)]
pub struct SchedulerEvent {
    pub task_id: Uuid,
    pub description: String,
    pub session_id: Option<Uuid>,
    pub label: Option<String>,
}

/// The cron + one-shot scheduler.
pub struct CronScheduler {
    tasks: Arc<TokioMutex<HashMap<Uuid, ScheduledTask>>>,
    event_tx: mpsc::Sender<SchedulerEvent>,
}

impl CronScheduler {
    /// Create a new scheduler. Returns the scheduler and a receiver for scheduler events.
    pub fn new() -> (Self, mpsc::Receiver<SchedulerEvent>) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let scheduler = Self {
            tasks: Arc::new(TokioMutex::new(HashMap::new())),
            event_tx,
        };
        (scheduler, event_rx)
    }

    /// Get a handle that can be used to add/remove tasks from other async contexts.
    pub fn handle(&self) -> SchedulerHandle {
        SchedulerHandle {
            tasks: self.tasks.clone(),
        }
    }

    /// Add a recurring cron task.
    pub async fn add_cron(
        &self,
        description: String,
        cron_expr: &str,
        label: Option<String>,
        session_id: Option<Uuid>,
    ) -> Result<Uuid, String> {
        // Validate the cron expression
        Schedule::from_str(cron_expr).map_err(|e| format!("Invalid cron expression: {e}"))?;

        let mut tasks = self.tasks.lock().await;

        // Deduplicate: skip if an active task with same label or same cron+description exists
        for existing in tasks.values() {
            if !existing.active {
                continue;
            }
            if let (Some(existing_label), Some(new_label)) = (&existing.label, &label) {
                if existing_label == new_label {
                    info!(task_id = %existing.id, label = %new_label, "cron task already exists — skipping");
                    return Ok(existing.id);
                }
            }
            if let ScheduleKind::Cron { expression: expr } = &existing.kind {
                if expr == cron_expr && existing.description == description {
                    info!(task_id = %existing.id, cron = cron_expr, "cron task already exists — skipping");
                    return Ok(existing.id);
                }
            }
        }

        let task = ScheduledTask {
            id: Uuid::new_v4(),
            label,
            description,
            kind: ScheduleKind::Cron {
                expression: cron_expr.to_string(),
            },
            created_at: Utc::now(),
            session_id,
            active: true,
            fire_count: 0,
            last_fired: None,
        };

        let id = task.id;
        tasks.insert(id, task);
        info!(task_id = %id, cron = cron_expr, "scheduled recurring task");
        Ok(id)
    }

    /// Add a one-shot delayed task.
    pub async fn add_one_shot(
        &self,
        description: String,
        delay_seconds: u64,
        label: Option<String>,
        session_id: Option<Uuid>,
    ) -> Uuid {
        let fire_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);
        let task = ScheduledTask {
            id: Uuid::new_v4(),
            label,
            description,
            kind: ScheduleKind::OneShot { fire_at },
            created_at: Utc::now(),
            session_id,
            active: true,
            fire_count: 0,
            last_fired: None,
        };

        let id = task.id;
        self.tasks.lock().await.insert(id, task);
        info!(
            task_id = %id,
            delay_secs = delay_seconds,
            fire_at = %fire_at,
            "scheduled one-shot task"
        );
        id
    }

    /// Remove a scheduled task.
    pub async fn remove(&self, task_id: Uuid) -> bool {
        self.tasks.lock().await.remove(&task_id).is_some()
    }

    /// List all active scheduled tasks.
    pub async fn list(&self) -> Vec<ScheduledTask> {
        self.tasks
            .lock()
            .await
            .values()
            .filter(|t| t.active)
            .cloned()
            .collect()
    }

    /// Load tasks from config (heartbeat_cron, goal crons).
    pub async fn load_from_config(&self, config: &claw_config::ClawConfig) {
        // Load heartbeat cron
        if let Some(ref cron_expr) = config.autonomy.heartbeat_cron {
            if config.autonomy.proactive {
                match self
                    .add_cron(
                        "Heartbeat: Check status of all active goals and continue any unfinished work. Review pending tasks, check for errors, and make progress on outstanding objectives.".to_string(),
                        cron_expr,
                        Some("heartbeat".to_string()),
                        None,
                    )
                    .await
                {
                    Ok(id) => info!(task_id = %id, cron = cron_expr, "loaded heartbeat cron from config"),
                    Err(e) => warn!(error = %e, cron = cron_expr, "failed to load heartbeat cron from config"),
                }
            }
        }

        // Load per-goal crons
        for goal_config in &config.autonomy.goals {
            if !goal_config.enabled {
                continue;
            }
            if let Some(ref cron_expr) = goal_config.cron {
                match self
                    .add_cron(
                        format!("Scheduled goal check: {}", goal_config.description),
                        cron_expr,
                        Some(goal_config.description.clone()),
                        None,
                    )
                    .await
                {
                    Ok(id) => info!(
                        task_id = %id,
                        goal = %goal_config.description,
                        cron = cron_expr,
                        "loaded goal cron from config"
                    ),
                    Err(e) => warn!(
                        error = %e,
                        goal = %goal_config.description,
                        "failed to load goal cron"
                    ),
                }
            }
        }
    }

    /// Run the scheduler loop. This should be spawned as a background task.
    /// Checks for due tasks every 10 seconds.
    pub async fn run(self) {
        let check_interval = tokio::time::Duration::from_secs(10);
        info!("scheduler started — checking every 10s");

        loop {
            tokio::time::sleep(check_interval).await;

            let now = Utc::now();
            let mut tasks = self.tasks.lock().await;
            let mut to_deactivate: Vec<Uuid> = Vec::new();

            for task in tasks.values_mut() {
                if !task.active {
                    continue;
                }

                let should_fire = match &task.kind {
                    ScheduleKind::Cron { expression } => {
                        match Schedule::from_str(expression) {
                            Ok(schedule) => {
                                // Check if there's a scheduled time between last_fired (or created_at) and now
                                let since = task.last_fired.unwrap_or(task.created_at);
                                schedule
                                    .after(&since)
                                    .take(1)
                                    .next()
                                    .is_some_and(|next| next <= now)
                            }
                            Err(e) => {
                                error!(task_id = %task.id, error = %e, "invalid cron expression — deactivating");
                                to_deactivate.push(task.id);
                                false
                            }
                        }
                    }
                    ScheduleKind::OneShot { fire_at } => now >= *fire_at,
                };

                if should_fire {
                    debug!(
                        task_id = %task.id,
                        label = ?task.label,
                        fire_count = task.fire_count + 1,
                        "scheduler firing task"
                    );

                    let event = SchedulerEvent {
                        task_id: task.id,
                        description: task.description.clone(),
                        session_id: task.session_id,
                        label: task.label.clone(),
                    };

                    if self.event_tx.send(event).await.is_err() {
                        warn!("scheduler event channel closed — shutting down");
                        return;
                    }

                    task.fire_count += 1;
                    task.last_fired = Some(now);

                    // One-shot tasks deactivate after firing
                    if matches!(task.kind, ScheduleKind::OneShot { .. }) {
                        to_deactivate.push(task.id);
                    }
                }
            }

            // Deactivate completed one-shots and broken crons
            for id in to_deactivate {
                if let Some(task) = tasks.get_mut(&id) {
                    task.active = false;
                    debug!(task_id = %id, "deactivated scheduled task");
                }
            }
        }
    }
}

/// A clone-able handle for adding tasks to the scheduler from other async contexts.
#[derive(Clone)]
pub struct SchedulerHandle {
    tasks: Arc<TokioMutex<HashMap<Uuid, ScheduledTask>>>,
}

impl SchedulerHandle {
    /// Add a recurring cron task.
    /// Deduplicates: if an active task with the same label or same (cron + description) exists,
    /// returns the existing task ID instead of creating a duplicate.
    pub async fn add_cron(
        &self,
        description: String,
        cron_expr: &str,
        label: Option<String>,
        session_id: Option<Uuid>,
    ) -> Result<Uuid, String> {
        Schedule::from_str(cron_expr).map_err(|e| format!("Invalid cron expression: {e}"))?;

        let mut tasks = self.tasks.lock().await;

        // Check for duplicate: same label (if provided) or same cron + description
        for existing in tasks.values() {
            if !existing.active {
                continue;
            }
            // Match by label if both have one
            if let (Some(existing_label), Some(new_label)) = (&existing.label, &label) {
                if existing_label == new_label {
                    info!(
                        task_id = %existing.id,
                        label = %new_label,
                        "cron task with same label already exists — skipping duplicate"
                    );
                    return Ok(existing.id);
                }
            }
            // Match by cron expression + description
            if let ScheduleKind::Cron { expression: expr } = &existing.kind {
                if expr == cron_expr && existing.description == description {
                    info!(
                        task_id = %existing.id,
                        cron = cron_expr,
                        "cron task with same expression and description already exists — skipping duplicate"
                    );
                    return Ok(existing.id);
                }
            }
        }

        let task = ScheduledTask {
            id: Uuid::new_v4(),
            label,
            description,
            kind: ScheduleKind::Cron {
                expression: cron_expr.to_string(),
            },
            created_at: Utc::now(),
            session_id,
            active: true,
            fire_count: 0,
            last_fired: None,
        };

        let id = task.id;
        tasks.insert(id, task);
        info!(task_id = %id, cron = cron_expr, "scheduled recurring task via handle");
        Ok(id)
    }

    /// Add a one-shot delayed task.
    pub async fn add_one_shot(
        &self,
        description: String,
        delay_seconds: u64,
        label: Option<String>,
        session_id: Option<Uuid>,
    ) -> Uuid {
        let fire_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);
        let task = ScheduledTask {
            id: Uuid::new_v4(),
            label,
            description,
            kind: ScheduleKind::OneShot { fire_at },
            created_at: Utc::now(),
            session_id,
            active: true,
            fire_count: 0,
            last_fired: None,
        };

        let id = task.id;
        self.tasks.lock().await.insert(id, task);
        info!(task_id = %id, delay_secs = delay_seconds, "scheduled one-shot task via handle");
        id
    }

    /// Remove a scheduled task.
    pub async fn remove(&self, task_id: Uuid) -> bool {
        self.tasks.lock().await.remove(&task_id).is_some()
    }

    /// Get a scheduled task by ID.
    pub async fn get(&self, task_id: Uuid) -> Option<ScheduledTask> {
        self.tasks.lock().await.get(&task_id).cloned()
    }

    /// List all scheduled tasks (active and inactive).
    pub async fn list_all(&self) -> Vec<ScheduledTask> {
        self.tasks.lock().await.values().cloned().collect()
    }

    /// Restore a previously-persisted task into the scheduler.
    /// Skips validation and deduplication — used at startup to reload from DB.
    pub async fn restore_task(&self, task: ScheduledTask) {
        let id = task.id;
        self.tasks.lock().await.insert(id, task);
    }
}
