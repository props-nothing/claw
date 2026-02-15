#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    // ── Episodic Memory ────────────────────────────────────────

    mod episodic {
        use super::*;
        use claw_memory::{Episode, EpisodicMemory};

        fn make_episode(summary: &str, tags: Vec<&str>) -> Episode {
            Episode {
                id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                summary: summary.to_string(),
                outcome: None,
                tags: tags.into_iter().map(String::from).collect(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }

        #[test]
        fn test_record_and_recent() {
            let mut mem = EpisodicMemory::new();
            mem.record(make_episode("first", vec!["a"]));
            mem.record(make_episode("second", vec!["b"]));
            mem.record(make_episode("third", vec!["c"]));
            let recent = mem.recent(2);
            assert_eq!(recent.len(), 2);
            assert_eq!(recent[0].summary, "second");
            assert_eq!(recent[1].summary, "third");
        }

        #[test]
        fn test_cap_at_100() {
            let mut mem = EpisodicMemory::new();
            for i in 0..110 {
                mem.record(make_episode(&format!("episode {i}"), vec![]));
            }
            let recent = mem.recent(200);
            assert_eq!(recent.len(), 100);
            // Oldest should have been evicted
            assert_eq!(recent[0].summary, "episode 10");
        }

        #[test]
        fn test_search_case_insensitive() {
            let mut mem = EpisodicMemory::new();
            mem.record(make_episode("Deployed to PRODUCTION", vec!["deploy"]));
            mem.record(make_episode("Fixed a bug", vec!["bugfix"]));
            mem.record(make_episode("Production rollback", vec![]));
            let results = mem.search("production");
            assert_eq!(results.len(), 2);
        }

        #[test]
        fn test_search_by_tag() {
            let mut mem = EpisodicMemory::new();
            mem.record(make_episode("task one", vec!["urgent"]));
            mem.record(make_episode("task two", vec!["low-priority"]));
            let results = mem.search("urgent");
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].summary, "task one");
        }

        #[test]
        fn test_for_session() {
            let mut mem = EpisodicMemory::new();
            let session = Uuid::new_v4();
            let mut ep = make_episode("in session", vec![]);
            ep.session_id = session;
            mem.record(ep);
            mem.record(make_episode("other session", vec![]));
            let results = mem.for_session(session);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].summary, "in session");
        }

        #[test]
        fn test_episode_serde() {
            let ep = make_episode("serde test", vec!["tag1", "tag2"]);
            let json = serde_json::to_string(&ep).unwrap();
            let restored: Episode = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.summary, "serde test");
            assert_eq!(restored.tags.len(), 2);
        }
    }

    // ── Semantic Memory ────────────────────────────────────────

    mod semantic {
        use super::*;
        use claw_memory::{Fact, SemanticMemory};

        fn make_fact(cat: &str, key: &str, val: &str) -> Fact {
            Fact {
                id: Uuid::new_v4(),
                category: cat.to_string(),
                key: key.to_string(),
                value: val.to_string(),
                confidence: 1.0,
                source: None,
                embedding: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }

        #[test]
        fn test_upsert_insert() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("user", "name", "Alice"));
            assert_eq!(mem.count(), 1);
            let fact = mem.get("user", "name").unwrap();
            assert_eq!(fact.value, "Alice");
        }

        #[test]
        fn test_upsert_update() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("user", "name", "Alice"));
            mem.upsert(make_fact("user", "name", "Bob"));
            assert_eq!(mem.count(), 1);
            let fact = mem.get("user", "name").unwrap();
            assert_eq!(fact.value, "Bob");
        }

        #[test]
        fn test_categories() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("user", "name", "Alice"));
            mem.upsert(make_fact("system", "os", "macOS"));
            let cats = mem.categories();
            assert_eq!(cats.len(), 2);
            assert!(cats.contains(&"user"));
            assert!(cats.contains(&"system"));
        }

        #[test]
        fn test_category_slice() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("prefs", "theme", "dark"));
            mem.upsert(make_fact("prefs", "language", "en"));
            mem.upsert(make_fact("other", "key", "val"));
            assert_eq!(mem.category("prefs").len(), 2);
            assert_eq!(mem.category("nonexistent").len(), 0);
        }

        #[test]
        fn test_search() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("user", "name", "Alice"));
            mem.upsert(make_fact("user", "email", "alice@example.com"));
            mem.upsert(make_fact("system", "hostname", "dev-box"));
            let results = mem.search("alice");
            assert_eq!(results.len(), 2);
        }

        #[test]
        fn test_vector_search() {
            let mut mem = SemanticMemory::new();
            let mut f1 = make_fact("embed", "a", "first");
            f1.embedding = Some(vec![1.0, 0.0, 0.0]);
            let mut f2 = make_fact("embed", "b", "second");
            f2.embedding = Some(vec![0.0, 1.0, 0.0]);
            let mut f3 = make_fact("embed", "c", "third");
            f3.embedding = Some(vec![0.9, 0.1, 0.0]);
            mem.upsert(f1);
            mem.upsert(f2);
            mem.upsert(f3);

            let query = vec![1.0, 0.0, 0.0];
            let results = mem.vector_search(&query, 2);
            assert_eq!(results.len(), 2);
            // f1 should be most similar (exact match)
            assert_eq!(results[0].0.key, "a");
            assert!((results[0].1 - 1.0).abs() < 0.01);
            // f3 should be second (0.9 cosine similarity)
            assert_eq!(results[1].0.key, "c");
        }

        #[test]
        fn test_all_facts() {
            let mut mem = SemanticMemory::new();
            mem.upsert(make_fact("a", "k1", "v1"));
            mem.upsert(make_fact("b", "k2", "v2"));
            assert_eq!(mem.all_facts().len(), 2);
        }

        #[test]
        fn test_fact_serde() {
            let fact = make_fact("cat", "key", "val");
            let json = serde_json::to_string(&fact).unwrap();
            let restored: Fact = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.category, "cat");
            assert_eq!(restored.key, "key");
        }
    }

    // ── Working Memory ─────────────────────────────────────────

    mod working {
        use super::*;
        use claw_core::{Message, Role};
        use claw_memory::WorkingMemory;

        #[test]
        fn test_session_creation() {
            let mut wm = WorkingMemory::new();
            let sid = Uuid::new_v4();
            let ctx = wm.session(sid);
            assert_eq!(ctx.session_id, sid);
            assert!(ctx.messages.is_empty());
            assert_eq!(ctx.max_tokens, 128_000);
        }

        #[test]
        fn test_push_and_messages() {
            let mut wm = WorkingMemory::new();
            let sid = Uuid::new_v4();
            wm.push(Message::text(sid, Role::User, "Hello"));
            wm.push(Message::text(sid, Role::Assistant, "Hi there!"));
            let msgs = wm.messages(sid);
            assert_eq!(msgs.len(), 2);
            assert_eq!(msgs[0].role, Role::User);
            assert_eq!(msgs[1].role, Role::Assistant);
        }

        #[test]
        fn test_messages_empty_for_unknown_session() {
            let wm = WorkingMemory::new();
            let msgs = wm.messages(Uuid::new_v4());
            assert!(msgs.is_empty());
        }

        #[test]
        fn test_compact_keeps_min_4() {
            let mut wm = WorkingMemory::new();
            let sid = Uuid::new_v4();
            // Push exactly 4 messages
            for i in 0..4 {
                wm.push(Message::text(sid, Role::User, format!("msg {i}")));
            }
            let result = wm.compact(sid);
            // Compaction should NOT reduce below 4
            assert!(result.is_none());
            assert_eq!(wm.messages(sid).len(), 4);
        }

        #[test]
        fn test_compact_reduces_messages() {
            let mut wm = WorkingMemory::new();
            let sid = Uuid::new_v4();
            for i in 0..20 {
                wm.push(Message::text(sid, Role::User, format!("message {i}")));
            }
            assert_eq!(wm.messages(sid).len(), 20);
            let summary = wm.compact(sid);
            assert!(summary.is_some());
            // After compaction: 1 pinned + 1 summary + keep_count (20/5 = 4) = 6
            let msgs = wm.messages(sid);
            assert!(msgs.len() < 20);
            assert!(msgs.len() >= 4);
            // First message is pinned (original user request), second is the compaction summary
            assert!(msgs[0].text_content().contains("message 0")); // pinned
            assert!(msgs[1].text_content().contains("Compacted")); // summary
        }

        #[test]
        fn test_clear() {
            let mut wm = WorkingMemory::new();
            let sid = Uuid::new_v4();
            wm.push(Message::text(sid, Role::User, "hello"));
            wm.clear(sid);
            assert!(wm.messages(sid).is_empty());
        }

        #[test]
        fn test_active_sessions() {
            let mut wm = WorkingMemory::new();
            let s1 = Uuid::new_v4();
            let s2 = Uuid::new_v4();
            wm.push(Message::text(s1, Role::User, "a"));
            wm.push(Message::text(s2, Role::User, "b"));
            let sessions = wm.active_sessions();
            assert_eq!(sessions.len(), 2);
        }
    }

    // ── Memory Store (SQLite roundtrip) ─────────────────────────

    mod store {
        use claw_memory::MemoryStore;

        #[test]
        fn test_open_creates_tables() {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.db");
            let store = MemoryStore::open(&db_path).unwrap();
            // Verify we can query the tables
            let db = store.db();
            let count: i64 = db
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            // At least episodes, facts, goals, goal_steps, audit_log
            assert!(count >= 5, "expected at least 5 tables, got {count}");
        }

        #[test]
        fn test_persist_and_load_facts() {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.db");
            let mut store = MemoryStore::open(&db_path).unwrap();
            store.persist_fact("user", "name", "Alice").unwrap();
            store
                .persist_fact("user", "email", "alice@test.com")
                .unwrap();

            // Reload facts from DB
            let count = store.load_facts().unwrap();
            // load_facts is also called in open(), but we added 2 more
            assert!(count >= 2);
            let fact = store.semantic.get("user", "name").unwrap();
            assert_eq!(fact.value, "Alice");
        }

        #[test]
        fn test_audit_log() {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.db");
            let store = MemoryStore::open(&db_path).unwrap();
            store
                .audit("tool_call", "shell_exec", Some("ls -la"))
                .unwrap();
            store.audit("approval", "approved", None).unwrap();

            let log = store.audit_log(10);
            assert_eq!(log.len(), 2);
            // Most recent first
            assert_eq!(log[0].1, "approval");
            assert_eq!(log[1].1, "tool_call");
        }

        #[test]
        fn test_persist_fact_upsert() {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.db");
            let store = MemoryStore::open(&db_path).unwrap();
            store.persist_fact("user", "name", "Alice").unwrap();
            store.persist_fact("user", "name", "Bob").unwrap();

            // Verify only one row with updated value
            let db = store.db();
            let val: String = db
                .query_row(
                    "SELECT value FROM facts WHERE category = 'user' AND key = 'name'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(val, "Bob");
        }
    }
}
