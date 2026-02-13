use claw_core::Message;
use uuid::Uuid;

/// Working memory — the current conversation context held in RAM.
///
/// This is what gets sent to the LLM as context. It manages window sizing,
/// compaction, and relevance scoring.
pub struct WorkingMemory {
    /// Active session messages.
    sessions: std::collections::HashMap<Uuid, SessionContext>,
}

/// Per-session working memory.
pub struct SessionContext {
    pub session_id: Uuid,
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
    /// Total estimated token count of this context.
    pub estimated_tokens: usize,
    /// Maximum tokens before we need to compact (model context window).
    pub max_tokens: usize,
    /// Compact at this fraction of max_tokens (e.g. 0.75).
    pub compaction_threshold: f64,
    /// Number of times this session has been compacted.
    pub compaction_count: u32,
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self {
            sessions: std::collections::HashMap::new(),
        }
    }

    /// Get or create a session context.
    pub fn session(&mut self, session_id: Uuid) -> &mut SessionContext {
        self.sessions.entry(session_id).or_insert_with(|| SessionContext {
            session_id,
            messages: Vec::new(),
            system_prompt: None,
            estimated_tokens: 0,
            max_tokens: 128_000,
            compaction_threshold: 0.75,
            compaction_count: 0,
        })
    }

    /// Configure the context window for a session.
    pub fn set_context_window(&mut self, session_id: Uuid, max_tokens: usize, threshold: f64) {
        let ctx = self.session(session_id);
        ctx.max_tokens = max_tokens;
        ctx.compaction_threshold = threshold.clamp(0.5, 0.95);
    }

    /// Add a message to a session.
    pub fn push(&mut self, message: Message) {
        let session_id = message.session_id;
        let ctx = self.session(session_id);
        let token_estimate = message.estimate_tokens();
        ctx.estimated_tokens += token_estimate;
        ctx.messages.push(message);

        // Auto-compact if over threshold
        let threshold = (ctx.max_tokens as f64 * ctx.compaction_threshold) as usize;
        if ctx.estimated_tokens > threshold {
            self.compact(session_id);
        }
    }

    /// Get all messages for a session (ready to send to LLM).
    pub fn messages(&self, session_id: Uuid) -> &[Message] {
        self.sessions
            .get(&session_id)
            .map(|ctx| ctx.messages.as_slice())
            .unwrap_or(&[])
    }

    /// Get the estimated token count for a session.
    pub fn token_count(&self, session_id: Uuid) -> usize {
        self.sessions
            .get(&session_id)
            .map(|ctx| ctx.estimated_tokens)
            .unwrap_or(0)
    }

    /// Check if a session needs compaction (is above threshold).
    pub fn needs_compaction(&self, session_id: Uuid) -> bool {
        if let Some(ctx) = self.sessions.get(&session_id) {
            let threshold = (ctx.max_tokens as f64 * ctx.compaction_threshold) as usize;
            ctx.estimated_tokens > threshold
        } else {
            false
        }
    }

    /// Compact a session by summarizing older messages.
    /// This is the "naive" compaction — concatenates old messages into a summary.
    /// The first User message (original request) is always preserved (pinned).
    /// For LLM-powered compaction, use `prepare_compaction_request` + `apply_compaction`.
    pub fn compact(&mut self, session_id: Uuid) -> Option<String> {
        let ctx = self.sessions.get_mut(&session_id)?;
        if ctx.messages.len() <= 4 {
            return None; // Too few to compact
        }

        // Pin: first User message is always kept
        let pin_count = ctx.messages.iter()
            .take(3) // only check first 3 messages
            .position(|m| m.role == claw_core::Role::User)
            .map(|i| i + 1) // include the pinned message
            .unwrap_or(0);

        // Keep the last 20% of messages (min 4) + pinned messages at the front
        let keep_tail = (ctx.messages.len() / 5).max(4);
        let to_summarize_end = ctx.messages.len() - keep_tail;

        // Don't summarize pinned messages
        let summarize_start = pin_count;
        if to_summarize_end <= summarize_start {
            return None; // Nothing to summarize
        }

        // Build a summary from the middle messages (between pinned and recent)
        let summary_parts: Vec<String> = ctx.messages[summarize_start..to_summarize_end]
            .iter()
            .map(|m| {
                let role = match m.role {
                    claw_core::Role::User => "User",
                    claw_core::Role::Assistant => "Assistant",
                    claw_core::Role::System => "System",
                    claw_core::Role::Tool => "Tool",
                };
                let text = m.text_content();
                let truncated: String = text.chars().take(500).collect();
                format!("[{}]: {}", role, truncated)
            })
            .collect();

        let messages_summarized = to_summarize_end - summarize_start;
        let summary = format!(
            "[Compacted {} earlier messages]\n{}",
            messages_summarized,
            summary_parts.join("\n").chars().take(2000).collect::<String>()
        );

        // Rebuild: pinned + summary + recent tail
        let pinned: Vec<Message> = ctx.messages[..pin_count].to_vec();
        let recent: Vec<Message> = ctx.messages[to_summarize_end..].to_vec();
        ctx.messages.clear();
        ctx.messages.extend(pinned);
        ctx.messages.push(Message::text(session_id, claw_core::Role::System, &summary));
        ctx.messages.extend(recent);

        // Recount tokens
        ctx.estimated_tokens = ctx.messages.iter().map(|m| m.estimate_tokens()).sum();
        ctx.compaction_count += 1;

        Some(summary)
    }

    /// Prepare a compaction request — returns the text that should be summarized
    /// by an LLM. The first User message (original request) is pinned and excluded.
    /// After getting the LLM summary, call `apply_llm_compaction()`.
    pub fn prepare_compaction_request(&self, session_id: Uuid) -> Option<(String, usize)> {
        let ctx = self.sessions.get(&session_id)?;
        if ctx.messages.len() <= 6 {
            return None;
        }

        // Pin: first User message is always kept
        let pin_count = ctx.messages.iter()
            .take(3)
            .position(|m| m.role == claw_core::Role::User)
            .map(|i| i + 1)
            .unwrap_or(0);

        // Identify messages to compact: everything between pinned and recent 30% (min 4)
        let keep_tail = (ctx.messages.len() * 3 / 10).max(4);
        let to_summarize_end = ctx.messages.len() - keep_tail;
        let summarize_start = pin_count;

        if to_summarize_end <= summarize_start {
            return None;
        }

        let messages_to_summarize = to_summarize_end - summarize_start;

        let mut compaction_text = String::new();
        for msg in &ctx.messages[summarize_start..to_summarize_end] {
            let role = match msg.role {
                claw_core::Role::User => "User",
                claw_core::Role::Assistant => "Assistant",
                claw_core::Role::System => "System",
                claw_core::Role::Tool => "Tool",
            };
            let text = msg.text_content();
            // Cap each message to 1000 chars in the compaction input
            let truncated: String = text.chars().take(1000).collect();
            if !truncated.is_empty() {
                compaction_text.push_str(&format!("[{}]: {}\n", role, truncated));
            }
            // Include tool calls
            for tc in &msg.tool_calls {
                compaction_text.push_str(&format!(
                    "[Tool Call]: {}({})\n",
                    tc.tool_name,
                    tc.arguments.to_string().chars().take(200).collect::<String>()
                ));
            }
        }

        Some((compaction_text, messages_to_summarize))
    }

    /// Apply an LLM-generated summary, replacing old messages with the summary.
    /// Preserves pinned messages (first User message) at the front.
    pub fn apply_llm_compaction(&mut self, session_id: Uuid, summary: &str, messages_to_remove: usize) {
        if let Some(ctx) = self.sessions.get_mut(&session_id) {
            // Pin: first User message is always kept
            let pin_count = ctx.messages.iter()
                .take(3)
                .position(|m| m.role == claw_core::Role::User)
                .map(|i| i + 1)
                .unwrap_or(0);

            let remove_end = pin_count + messages_to_remove;
            if remove_end >= ctx.messages.len() {
                return; // Safety check
            }

            // Rebuild: pinned + summary + recent tail
            let pinned: Vec<Message> = ctx.messages[..pin_count].to_vec();
            let recent: Vec<Message> = ctx.messages[remove_end..].to_vec();
            ctx.messages.clear();

            ctx.messages.extend(pinned);

            // Insert the LLM summary as a system message
            let summary_msg = Message::text(
                session_id,
                claw_core::Role::System,
                &format!("[Conversation summary — compacted {} messages]\n{}", messages_to_remove, summary),
            );
            ctx.messages.push(summary_msg);
            ctx.messages.extend(recent);

            // Recount tokens accurately
            ctx.estimated_tokens = ctx.messages.iter().map(|m| m.estimate_tokens()).sum();
            ctx.compaction_count += 1;
        }
    }

    /// Force recount tokens (useful after external modifications).
    pub fn recount_tokens(&mut self, session_id: Uuid) {
        if let Some(ctx) = self.sessions.get_mut(&session_id) {
            ctx.estimated_tokens = ctx.messages.iter().map(|m| m.estimate_tokens()).sum();
        }
    }

    /// Clear a session's working memory.
    pub fn clear(&mut self, session_id: Uuid) {
        self.sessions.remove(&session_id);
    }

    /// List active session IDs.
    pub fn active_sessions(&self) -> Vec<Uuid> {
        self.sessions.keys().copied().collect()
    }
}
