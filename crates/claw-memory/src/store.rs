use std::path::Path;
use std::sync::Arc;
use rusqlite::Connection;
use parking_lot::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::episodic::EpisodicMemory;
use crate::semantic::SemanticMemory;
use crate::working::WorkingMemory;

/// Unified memory store combining all three memory tiers.
pub struct MemoryStore {
    pub working: WorkingMemory,
    pub episodic: EpisodicMemory,
    pub semantic: SemanticMemory,
    db: Arc<Mutex<Connection>>,
}

impl MemoryStore {
    /// Open or create the memory database at the given path.
    pub fn open(path: &Path) -> claw_core::Result<Self> {
        info!(?path, "opening memory store");

        let conn = Connection::open(path)
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        // Enable WAL mode for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        // Create tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS episodes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                outcome TEXT,
                tags TEXT DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS episode_messages (
                id TEXT PRIMARY KEY,
                episode_id TEXT NOT NULL REFERENCES episodes(id),
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS facts (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL DEFAULT 1.0,
                source TEXT,
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(category, key)
            );

            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                priority INTEGER DEFAULT 5,
                progress REAL DEFAULT 0.0,
                parent_id TEXT REFERENCES goals(id),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS goal_steps (
                id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL REFERENCES goals(id),
                description TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                result TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                event_type TEXT NOT NULL,
                action TEXT NOT NULL,
                details TEXT,
                checksum TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);
            CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                name TEXT,
                channel TEXT,
                target TEXT,
                active INTEGER DEFAULT 1,
                message_count INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_active ON sessions(active);

            CREATE TABLE IF NOT EXISTS session_messages (
                session_id TEXT PRIMARY KEY,
                messages_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let db = Arc::new(Mutex::new(conn));

        let mut episodic = EpisodicMemory::new();
        episodic.set_db(Arc::clone(&db));

        let mut store = Self {
            working: WorkingMemory::new(),
            episodic,
            semantic: SemanticMemory::new(),
            db,
        };

        // Load persisted data on startup
        match store.load_facts() {
            Ok(count) => if count > 0 { info!(count, "loaded facts from SQLite"); },
            Err(e) => tracing::warn!(error = %e, "failed to load facts from SQLite"),
        }
        match store.episodic.load_from_db() {
            Ok(count) => if count > 0 { info!(count, "loaded episodes from SQLite"); },
            Err(e) => tracing::warn!(error = %e, "failed to load episodes from SQLite"),
        }

        Ok(store)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> claw_core::Result<Self> {
        use std::path::Path;
        Self::open(Path::new(":memory:"))
    }

    /// Get a reference to the raw database connection (for advanced queries).
    pub fn db(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.db.lock()
    }

    /// Persist a fact to SQLite (upsert by category+key), optionally with an embedding.
    pub fn persist_fact(
        &self,
        category: &str,
        key: &str,
        value: &str,
    ) -> claw_core::Result<()> {
        self.persist_fact_with_embedding(category, key, value, None)
    }

    /// Persist a fact with an optional embedding vector.
    pub fn persist_fact_with_embedding(
        &self,
        category: &str,
        key: &str,
        value: &str,
        embedding: Option<&[f32]>,
    ) -> claw_core::Result<()> {
        let db = self.db.lock();
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let embedding_blob: Option<Vec<u8>> = embedding.map(|emb| {
            emb.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        db.execute(
            "INSERT INTO facts (id, category, key, value, confidence, source, embedding, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1.0, 'agent', ?5, ?6, ?6)
             ON CONFLICT(category, key) DO UPDATE SET value = excluded.value, embedding = COALESCE(excluded.embedding, facts.embedding), updated_at = excluded.updated_at",
            rusqlite::params![id, category, key, value, embedding_blob, now],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Delete a fact from SQLite by category and key.
    pub fn delete_fact(&self, category: &str, key: &str) -> claw_core::Result<bool> {
        let db = self.db.lock();
        let rows = db.execute(
            "DELETE FROM facts WHERE category = ?1 AND key = ?2",
            rusqlite::params![category, key],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(rows > 0)
    }

    /// Delete all facts in a category from SQLite. Returns number of rows deleted.
    pub fn delete_facts_by_category(&self, category: &str) -> claw_core::Result<usize> {
        let db = self.db.lock();
        let rows = db.execute(
            "DELETE FROM facts WHERE category = ?1",
            rusqlite::params![category],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(rows)
    }

    /// Load all facts from SQLite into semantic memory. Returns number of facts loaded.
    pub fn load_facts(&mut self) -> claw_core::Result<usize> {
        let rows: Vec<(String, String, String, f64, Option<Vec<u8>>)> = {
            let db = self.db.lock();
            let mut stmt = db
                .prepare("SELECT category, key, value, confidence, embedding FROM facts")
                .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                    ))
                })
                .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            rows
        };

        let count = rows.len();
        for (category, key, value, confidence, embedding_blob) in rows {
            // Deserialize embedding from LE f32 bytes
            let embedding = embedding_blob.and_then(|blob| {
                if blob.len() % 4 != 0 { return None; }
                Some(
                    blob.chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                        .collect::<Vec<f32>>()
                )
            });

            let fact = crate::semantic::Fact {
                id: Uuid::new_v4(),
                category,
                key,
                value,
                confidence,
                source: Some("sqlite".to_string()),
                embedding,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.semantic.upsert(fact);
        }
        Ok(count)
    }

    /// Write an audit log entry with a tamper-evident checksum.
    pub fn audit(
        &self,
        event_type: &str,
        action: &str,
        details: Option<&str>,
    ) -> claw_core::Result<()> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let checksum_input = format!("{}:{}:{}:{}", timestamp, event_type, action, details.unwrap_or(""));
        // Simple checksum — production would use HMAC with a device-local key
        let checksum = format!("{:x}", md5_hash(checksum_input.as_bytes()));

        let db = self.db.lock();
        db.execute(
            "INSERT INTO audit_log (timestamp, event_type, action, details, checksum) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![timestamp, event_type, action, details, checksum],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        Ok(())
    }

    /// Read recent audit log entries.
    pub fn audit_log(&self, limit: usize) -> Vec<(String, String, String, Option<String>)> {
        let db = self.db.lock();
        let mut stmt = match db.prepare(
            "SELECT timestamp, event_type, action, details FROM audit_log ORDER BY id DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(rusqlite::params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Persist a goal to SQLite (upsert by id).
    pub fn persist_goal(
        &self,
        id: &Uuid,
        description: &str,
        status: &str,
        priority: u8,
        progress: f32,
        parent_id: Option<&Uuid>,
    ) -> claw_core::Result<()> {
        let db = self.db.lock();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO goals (id, description, status, priority, progress, parent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                status = excluded.status,
                priority = excluded.priority,
                progress = excluded.progress,
                updated_at = excluded.updated_at",
            rusqlite::params![
                id.to_string(),
                description,
                status,
                priority as i32,
                progress as f64,
                parent_id.map(|p| p.to_string()),
                now,
            ],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Persist a goal step to SQLite (upsert by id).
    pub fn persist_goal_step(
        &self,
        step_id: &Uuid,
        goal_id: &Uuid,
        description: &str,
        status: &str,
        result: Option<&str>,
    ) -> claw_core::Result<()> {
        let db = self.db.lock();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO goal_steps (id, goal_id, description, status, result, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                result = excluded.result",
            rusqlite::params![
                step_id.to_string(),
                goal_id.to_string(),
                description,
                status,
                result,
                now,
            ],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Load all goals and their steps from SQLite. Returns the raw data as tuples
    /// (id, description, status, priority, progress, parent_id).
    pub fn load_goals(&self) -> claw_core::Result<Vec<GoalRow>> {
        let db = self.db.lock();
        let mut stmt = db
            .prepare(
                "SELECT g.id, g.description, g.status, g.priority, g.progress, g.parent_id
                 FROM goals g
                 ORDER BY g.priority DESC"
            )
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let goals: Vec<GoalRow> = stmt
            .query_map([], |row| {
                let goal_id: String = row.get(0)?;

                Ok(GoalRow {
                    id: goal_id,
                    description: row.get(1)?,
                    status: row.get(2)?,
                    priority: row.get::<_, i32>(3)? as u8,
                    progress: row.get::<_, f64>(4)? as f32,
                    parent_id: row.get(5)?,
                    steps: Vec::new(),
                })
            })
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        // Load steps for each goal
        let mut result = goals;
        for goal in &mut result {
            let mut step_stmt = db
                .prepare(
                    "SELECT id, description, status, result FROM goal_steps WHERE goal_id = ?1 ORDER BY created_at"
                )
                .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

            goal.steps = step_stmt
                .query_map(rusqlite::params![goal.id], |row| {
                    Ok(GoalStepRow {
                        id: row.get(0)?,
                        description: row.get(1)?,
                        status: row.get(2)?,
                        result: row.get(3)?,
                    })
                })
                .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();
        }

        Ok(result)
    }
}

/// A raw goal row loaded from SQLite.
#[derive(Debug, Clone)]
pub struct GoalRow {
    pub id: String,
    pub description: String,
    pub status: String,
    pub priority: u8,
    pub progress: f32,
    pub parent_id: Option<String>,
    pub steps: Vec<GoalStepRow>,
}

/// A raw goal step row loaded from SQLite.
#[derive(Debug, Clone)]
pub struct GoalStepRow {
    pub id: String,
    pub description: String,
    pub status: String,
    pub result: Option<String>,
}

/// A raw session row loaded from SQLite.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub name: Option<String>,
    pub channel: Option<String>,
    pub target: Option<String>,
    pub active: bool,
    pub message_count: usize,
    pub created_at: String,
}

impl MemoryStore {
    /// Persist a session to SQLite (upsert by id).
    pub fn persist_session(
        &self,
        id: &Uuid,
        name: Option<&str>,
        channel: Option<&str>,
        target: Option<&str>,
        active: bool,
        message_count: usize,
    ) -> claw_core::Result<()> {
        let db = self.db.lock();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO sessions (id, name, channel, target, active, message_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                active = excluded.active,
                message_count = excluded.message_count,
                updated_at = excluded.updated_at",
            rusqlite::params![
                id.to_string(),
                name,
                channel,
                target,
                active as i32,
                message_count as i64,
                now,
            ],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Load sessions from SQLite. Returns recent active sessions.
    pub fn load_sessions(&self, limit: usize) -> claw_core::Result<Vec<SessionRow>> {
        let db = self.db.lock();
        let mut stmt = db
            .prepare(
                "SELECT id, name, channel, target, active, message_count, created_at
                 FROM sessions
                 ORDER BY updated_at DESC
                 LIMIT ?1"
            )
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    channel: row.get(2)?,
                    target: row.get(3)?,
                    active: row.get::<_, i32>(4)? != 0,
                    message_count: row.get::<_, i64>(5)? as usize,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Delete empty sessions (0 messages, no name) from SQLite to prevent clutter.
    pub fn cleanup_empty_sessions(&self) -> claw_core::Result<usize> {
        let db = self.db.lock();
        let deleted = db.execute(
            "DELETE FROM sessions WHERE message_count = 0",
            [],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        // Also clean up orphaned session_messages
        let _ = db.execute(
            "DELETE FROM session_messages WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
        );
        Ok(deleted)
    }

    /// Persist session messages (working memory) to SQLite as a JSON blob.
    pub fn persist_session_messages(
        &self,
        session_id: &Uuid,
        messages: &[claw_core::Message],
    ) -> claw_core::Result<()> {
        let json = serde_json::to_string(messages)
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        let db = self.db.lock();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO session_messages (session_id, messages_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                messages_json = excluded.messages_json,
                updated_at = excluded.updated_at",
            rusqlite::params![session_id.to_string(), json, now],
        )
        .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Load session messages from SQLite.
    pub fn load_session_messages(
        &self,
        session_id: &Uuid,
    ) -> claw_core::Result<Vec<claw_core::Message>> {
        let db = self.db.lock();
        let mut stmt = db
            .prepare("SELECT messages_json FROM session_messages WHERE session_id = ?1")
            .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;

        let json: Option<String> = stmt
            .query_row(rusqlite::params![session_id.to_string()], |row| row.get(0))
            .ok();

        match json {
            Some(j) => {
                let messages: Vec<claw_core::Message> = serde_json::from_str(&j)
                    .map_err(|e| claw_core::ClawError::Memory(e.to_string()))?;
                Ok(messages)
            }
            None => Ok(Vec::new()),
        }
    }
}

/// Simple hash for audit checksums (would use blake3 or HMAC in production).
fn md5_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}
