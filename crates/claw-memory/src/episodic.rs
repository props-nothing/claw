use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// An episode is a summarized record of a past conversation or task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub session_id: Uuid,
    pub summary: String,
    pub outcome: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Manages episodic memory — past interactions the agent can recall.
pub struct EpisodicMemory {
    /// In-memory cache of recent episodes for fast access.
    recent: Vec<Episode>,
    /// Shared database connection for persistence.
    db: Option<Arc<Mutex<Connection>>>,
}

impl EpisodicMemory {
    pub fn new() -> Self {
        Self {
            recent: Vec::new(),
            db: None,
        }
    }

    /// Set the shared database connection for persistence.
    pub fn set_db(&mut self, db: Arc<Mutex<Connection>>) {
        self.db = Some(db);
    }

    /// Record a new episode (in-memory + SQLite).
    pub fn record(&mut self, episode: Episode) {
        // Persist to SQLite
        if let Some(ref db) = self.db {
            let db = db.lock();
            let tags_json =
                serde_json::to_string(&episode.tags).unwrap_or_else(|_| "[]".to_string());
            let _ = db.execute(
                "INSERT OR REPLACE INTO episodes (id, session_id, summary, outcome, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    episode.id.to_string(),
                    episode.session_id.to_string(),
                    &episode.summary,
                    &episode.outcome,
                    &tags_json,
                    episode.created_at.to_rfc3339(),
                    episode.updated_at.to_rfc3339(),
                ],
            );
        }

        self.recent.push(episode);
        // Keep only the most recent N episodes in memory
        if self.recent.len() > 100 {
            self.recent.remove(0);
        }
    }

    /// Load recent episodes from SQLite into memory.
    pub fn load_from_db(&mut self) -> claw_core::Result<usize> {
        let Some(ref db) = self.db else {
            return Ok(0);
        };
        let db = db.lock();
        let mut stmt = db
            .prepare("SELECT id, session_id, summary, outcome, tags, created_at, updated_at FROM episodes ORDER BY created_at DESC LIMIT 100")
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let session_str: String = row.get(1)?;
                let summary: String = row.get(2)?;
                let outcome: Option<String> = row.get(3)?;
                let tags_str: String = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                Ok((
                    id_str,
                    session_str,
                    summary,
                    outcome,
                    tags_str,
                    created_str,
                    updated_str,
                ))
            })
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let mut count = 0;
        for row in rows {
            if let Ok((id_str, session_str, summary, outcome, tags_str, created_str, updated_str)) =
                row
            {
                let id = id_str.parse::<Uuid>().unwrap_or_else(|_| Uuid::new_v4());
                let session_id = session_str.parse::<Uuid>().unwrap_or_else(|_| Uuid::nil());
                let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                self.recent.push(Episode {
                    id,
                    session_id,
                    summary,
                    outcome,
                    tags,
                    created_at,
                    updated_at,
                });
                count += 1;
            }
        }
        // Reverse so oldest is first (we loaded DESC)
        self.recent.reverse();
        Ok(count)
    }

    /// Search episodes by keyword.
    pub fn search(&self, query: &str) -> Vec<&Episode> {
        let query_lower = query.to_lowercase();
        self.recent
            .iter()
            .filter(|e| {
                e.summary.to_lowercase().contains(&query_lower)
                    || e.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// Get the N most recent episodes.
    pub fn recent(&self, n: usize) -> &[Episode] {
        let start = self.recent.len().saturating_sub(n);
        &self.recent[start..]
    }

    /// Get all episodes for a session.
    pub fn for_session(&self, session_id: Uuid) -> Vec<&Episode> {
        self.recent
            .iter()
            .filter(|e| e.session_id == session_id)
            .collect()
    }
}
