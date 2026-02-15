use chrono;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, RwLock};
use uuid::Uuid;

/// A conversation session.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub name: Option<String>,
    /// Source channel + chat/user ID.
    pub channel: Option<String>,
    pub target: Option<String>,
    /// Whether the session is active.
    pub active: bool,
    /// Number of messages in this session.
    pub message_count: usize,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            channel: None,
            target: None,
            active: true,
            message_count: 0,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn with_channel(mut self, channel: &str, target: &str) -> Self {
        self.channel = Some(channel.to_string());
        self.target = Some(target.to_string());
        self
    }
}

/// Manages all active sessions.
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<Uuid, Session>>>,
    /// Per-session run locks — prevents concurrent agent runs on the same session.
    run_locks: Arc<RwLock<HashMap<Uuid, Arc<TokioMutex<()>>>>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            run_locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create(&self) -> Uuid {
        let session = Session::new();
        let id = session.id;
        self.sessions.write().await.insert(id, session);
        id
    }

    pub async fn create_for_channel(&self, channel: &str, target: &str) -> Uuid {
        let session = Session::new().with_channel(channel, target);
        let id = session.id;
        self.sessions.write().await.insert(id, session);
        id
    }

    pub async fn get(&self, id: Uuid) -> Option<Session> {
        let sessions = self.sessions.read().await;
        sessions.get(&id).cloned()
    }

    /// Increment message count for a session.
    pub async fn record_message(&self, id: Uuid) {
        if let Some(session) = self.sessions.write().await.get_mut(&id) {
            session.message_count += 1;
        }
    }

    /// Get the number of active sessions.
    pub async fn active_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| s.active)
            .count()
    }

    /// List all sessions (cloned).
    pub async fn list_sessions(&self) -> Vec<Session> {
        self.sessions.read().await.values().cloned().collect()
    }

    /// Find a session for a given channel + target, or create one.
    pub async fn find_or_create(&self, channel: &str, target: &str) -> Uuid {
        let sessions = self.sessions.read().await;
        for (id, session) in sessions.iter() {
            if session.channel.as_deref() == Some(channel)
                && session.target.as_deref() == Some(target)
                && session.active
            {
                return *id;
            }
        }
        drop(sessions);
        self.create_for_channel(channel, target).await
    }

    /// Look up a session by ID. If it exists, ensure it has channel/target set.
    /// If it doesn't exist, create one with the given channel/target.
    pub async fn get_or_insert(&self, id: Uuid, channel: &str, target: &str) -> Uuid {
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&id) {
                // Backfill channel/target if missing
                if session.channel.is_none() {
                    session.channel = Some(channel.to_string());
                }
                if session.target.is_none() {
                    session.target = Some(target.to_string());
                }
                return id;
            }
        }
        // Session doesn't exist, create it with the given ID
        let session = Session {
            id,
            name: None,
            channel: Some(channel.to_string()),
            target: Some(target.to_string()),
            active: true,
            message_count: 0,
            created_at: chrono::Utc::now(),
        };
        self.sessions.write().await.insert(id, session);
        id
    }

    pub async fn list(&self) -> Vec<Uuid> {
        self.sessions.read().await.keys().copied().collect()
    }

    pub async fn close(&self, id: Uuid) {
        if let Some(session) = self.sessions.write().await.get_mut(&id) {
            session.active = false;
        }
    }

    /// Set a session's display name / label.
    pub async fn set_name(&self, id: Uuid, name: &str) {
        if let Some(session) = self.sessions.write().await.get_mut(&id) {
            session.name = Some(name.to_string());
        }
    }

    /// Restore a session from persistent storage.
    pub async fn restore(
        &self,
        id: Uuid,
        name: Option<String>,
        channel: Option<String>,
        target: Option<String>,
        message_count: usize,
    ) {
        let session = Session {
            id,
            name,
            channel,
            target,
            active: true,
            message_count,
            created_at: chrono::Utc::now(),
        };
        self.sessions.write().await.insert(id, session);
    }

    /// Get all sessions for persistence (snapshot).
    pub async fn snapshot(&self) -> Vec<Session> {
        self.sessions.read().await.values().cloned().collect()
    }

    /// Get the per-session run lock. Callers should hold the guard for the
    /// duration of their agent loop to prevent concurrent runs on the same session.
    pub async fn run_lock(&self, session_id: Uuid) -> Arc<TokioMutex<()>> {
        // Fast path: lock already exists
        {
            let locks = self.run_locks.read().await;
            if let Some(lock) = locks.get(&session_id) {
                return Arc::clone(lock);
            }
        }
        // Slow path: create a new lock
        let mut locks = self.run_locks.write().await;
        Arc::clone(
            locks
                .entry(session_id)
                .or_insert_with(|| Arc::new(TokioMutex::new(()))),
        )
    }
}
