use uuid::Uuid;
use tracing::{debug, info, warn};
use claw_core::{Message, Role};
use claw_llm::LlmRequest;
use claw_mesh::MeshMessage;
use crate::agent::SharedAgentState;

/// Build a brief episodic summary from the conversation messages.
pub(crate) fn build_episode_summary(messages: &[Message], user_text: &str, final_response: &str) -> String {
    // Count tool calls across all messages
    let tool_names: Vec<String> = messages
        .iter()
        .flat_map(|m| m.tool_calls.iter().map(|tc| tc.tool_name.clone()))
        .collect();
    let tool_count = tool_names.len();

    let user_preview: String = user_text.chars().take(120).collect();
    let response_preview: String = final_response.chars().take(200).collect();

    let mut summary = format!("User: {user_preview}");
    if tool_count > 0 {
        let unique_tools: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            tool_names
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect()
        };
        summary.push_str(&format!(
            " | Tools used ({}): {}",
            tool_count,
            unique_tools.join(", ")
        ));
    }
    if !response_preview.is_empty() {
        summary.push_str(&format!(" | Response: {response_preview}"));
    }
    summary
}

/// Extract simple keyword tags from user text for episodic search.
pub(crate) fn extract_episode_tags(user_text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let lower = user_text.to_lowercase();

    // Detect broad task categories
    let categories = [
        (
            "code",
            &[
                "code",
                "program",
                "function",
                "bug",
                "fix",
                "implement",
                "build",
                "compile",
            ][..],
        ),
        (
            "file",
            &["file", "create", "write", "read", "edit", "delete"][..],
        ),
        (
            "web",
            &[
                "website",
                "web",
                "html",
                "css",
                "javascript",
                "react",
                "next",
            ][..],
        ),
        (
            "research",
            &["research", "search", "find", "look up", "explain"][..],
        ),
        (
            "deploy",
            &["deploy", "docker", "server", "host", "production"][..],
        ),
        ("config", &["config", "setup", "install", "configure"][..]),
        ("goal", &["goal", "plan", "task", "project"][..]),
    ];

    for (tag, keywords) in &categories {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            tags.push(tag.to_string());
        }
    }

    // Cap at 5 tags
    tags.truncate(5);
    tags
}

/// Extract meaningful search keywords from user text for memory retrieval.
/// Strips common filler words and short words, keeps domain-specific terms.
pub(crate) fn extract_search_keywords(user_text: &str) -> String {
    // Common stop words to filter out
    const STOP_WORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must", "can",
        "could", "i", "me", "my", "myself", "we", "our", "ours", "you", "your", "yours", "he",
        "him", "his", "she", "her", "hers", "it", "its", "they", "them", "their", "what", "which",
        "who", "whom", "this", "that", "these", "those", "am", "and", "but", "or", "nor", "not",
        "no", "so", "if", "then", "than", "too", "very", "just", "don", "now", "here", "there",
        "how", "all", "each", "every", "both", "few", "more", "most", "some", "any", "such",
        "only", "own", "same", "also", "into", "from", "with", "for", "on", "at", "to", "of", "in",
        "by", "up", "about", "out", "off", "over", "under", "again", "further", "once", "where",
        "when", "why", "after", "before", "above", "below", "between", "please", "want", "need",
        "help", "like", "make", "let", "get", "know", "think", "tell", "show", "give", "use",
        // Dutch stop words (since user communicates in Dutch)
        "de", "het", "een", "en", "van", "ik", "te", "dat", "die", "er", "zijn", "op", "aan", "met",
        "als", "voor", "nog", "maar", "om", "ook", "dan", "wel", "niet", "wat", "kun", "je", "kan",
        "naar", "hoe", "dit", "bij", "uit", "zo", "mijn", "jij", "wij", "zij",
    ];

    let lower = user_text.to_lowercase();
    let keywords: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
        .filter(|w| w.len() >= 3)
        .filter(|w| !STOP_WORDS.contains(w))
        .collect();

    // Also preserve URLs, email-like patterns, and dotted identifiers from original
    let mut extra: Vec<&str> = Vec::new();
    for word in user_text.split_whitespace() {
        if word.contains("://") || word.contains('.') && word.len() > 5 {
            extra.push(word);
        }
    }

    let mut combined: Vec<String> = keywords.iter().map(|s| s.to_string()).collect();
    for e in extra {
        let el = e.to_lowercase();
        if !combined.contains(&el) {
            combined.push(el);
        }
    }

    combined.join(" ")
}

// ── Self-Learning: Lesson Detection & Extraction ─────────────────────────

/// Detect whether the conversation contains error→correction→success patterns
/// that indicate a lesson was learned. Returns true if lesson extraction should run.
fn detect_lesson_patterns(messages: &[Message]) -> bool {
    // We look for patterns like:
    //   1. Tool result with is_error=true (something failed)
    //   2. Followed by a User message (correction/guidance)
    //   3. Followed by successful tool calls or assistant response
    //
    // OR:
    //   1. Assistant refuses/says it can't do something
    //   2. User corrects/insists
    //   3. Assistant succeeds

    let mut saw_error_or_refusal = false;
    let mut saw_user_correction_after = false;
    let mut saw_success_after = false;

    for msg in messages {
        match msg.role {
            Role::Tool => {
                for content in &msg.content {
                    if let claw_core::MessageContent::ToolResult { is_error, .. } = content {
                        if *is_error {
                            saw_error_or_refusal = true;
                            saw_user_correction_after = false;
                            saw_success_after = false;
                        }
                    }
                }
            }
            Role::Assistant => {
                let text = msg.text_content().to_lowercase();
                // Detect refusal patterns
                if text.contains("kan niet")
                    || text.contains("cannot")
                    || text.contains("can't")
                    || text.contains("mag niet")
                    || text.contains("unable to")
                    || text.contains("not able")
                    || text.contains("not allowed")
                    || text.contains("weiger")
                {
                    saw_error_or_refusal = true;
                    saw_user_correction_after = false;
                    saw_success_after = false;
                }
                // If we had an error + user correction, and now assistant acts, it's a success
                if saw_user_correction_after && !msg.tool_calls.is_empty() {
                    saw_success_after = true;
                }
            }
            Role::User => {
                if saw_error_or_refusal {
                    saw_user_correction_after = true;
                }
            }
            _ => {}
        }
    }

    // Pattern detected: error/refusal → user correction → continued work
    saw_error_or_refusal && (saw_user_correction_after || saw_success_after)
}

/// Build a conversation excerpt focusing on the error→correction→success patterns
/// for the LLM to analyze. Keeps it concise to minimize token usage.
fn build_lesson_excerpt(messages: &[Message]) -> String {
    let mut excerpt = String::new();
    let mut interesting_range = false;

    for msg in messages {
        let role_str = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => continue, // skip system messages
        };

        let text = msg.text_content();
        let is_error_result = msg.content.iter().any(
            |c| matches!(c, claw_core::MessageContent::ToolResult { is_error, .. } if *is_error),
        );

        // Start including messages when we hit an error
        if is_error_result {
            interesting_range = true;
        }
        let text_lower = text.to_lowercase();
        if msg.role == Role::Assistant
            && (text_lower.contains("kan niet")
                || text_lower.contains("cannot")
                || text_lower.contains("can't")
                || text_lower.contains("unable")
                || text_lower.contains("not allowed")
                || text_lower.contains("weiger"))
        {
            interesting_range = true;
        }

        if interesting_range {
            let truncated: String = text.chars().take(500).collect();
            if !truncated.is_empty() {
                excerpt.push_str(&format!("[{role_str}]: {truncated}\n"));
            }
            for tc in &msg.tool_calls {
                let args_preview: String = tc.arguments.to_string().chars().take(200).collect();
                excerpt.push_str(&format!(
                    "[Tool Call]: {}({})\n",
                    tc.tool_name, args_preview
                ));
            }
        }
    }

    // Cap the excerpt at ~4000 chars to keep the LLM call cheap
    excerpt.chars().take(4000).collect()
}

/// Use the LLM to extract lessons from a conversation where errors were corrected.
/// Returns a list of (key, lesson_text) tuples to store in semantic memory.
async fn extract_lessons_via_llm(
    state: &SharedAgentState,
    messages: &[Message],
) -> Vec<(String, String)> {
    let excerpt = build_lesson_excerpt(messages);
    if excerpt.is_empty() {
        return vec![];
    }

    let prompt = format!(
        "Analyze this conversation excerpt where mistakes were made and then corrected. \
         Extract specific, actionable lessons learned.\n\n\
         For each lesson, output a JSON array of objects with:\n\
         - \"key\": a short snake_case identifier (e.g. \"plesk_login_needs_cookie_accept\")\n\
         - \"lesson\": a concise description of what was learned, including the correct approach\n\n\
         Focus on:\n\
         - Technical procedures that were figured out through trial and error\n\
         - Form fields that needed specific values (IDs vs text, hidden fields, etc.)\n\
         - Steps that must be done in a specific order\n\
         - Common errors and their solutions\n\
         - Anything the user had to correct or point out\n\n\
         Output ONLY the JSON array, nothing else. If no clear lessons, output [].\n\n\
         Conversation excerpt:\n{excerpt}\n"
    );

    // Use fast model if available to keep costs low
    let model = state
        .config
        .agent
        .fast_model
        .as_deref()
        .unwrap_or(&state.config.agent.model);

    let request = LlmRequest {
        model: model.to_string(),
        messages: vec![Message::text(Uuid::nil(), Role::User, &prompt)],
        tools: vec![],
        system: Some("You are a precise lesson extractor. Output only valid JSON.".to_string()),
        max_tokens: 1024,
        temperature: 0.2,
        thinking_level: Some("off".to_string()),
        stream: false,
    };

    match state.llm.complete(&request, None).await {
        Ok(response) => {
            let text = response.message.text_content();
            // Parse JSON from the response (handle markdown code fences)
            let json_text = text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            match serde_json::from_str::<Vec<serde_json::Value>>(json_text) {
                Ok(items) => items
                    .iter()
                    .filter_map(|item| {
                        let key = item["key"].as_str()?.to_string();
                        let lesson = item["lesson"].as_str()?.to_string();
                        Some((key, lesson))
                    })
                    .collect(),
                Err(e) => {
                    debug!(error = %e, "failed to parse lesson extraction JSON");
                    vec![]
                }
            }
        }
        Err(e) => {
            debug!(error = %e, "lesson extraction LLM call failed");
            vec![]
        }
    }
}

/// Run the full self-learning pipeline: detect patterns, extract lessons, persist them.
pub(crate) async fn maybe_extract_lessons(state: &SharedAgentState, session_id: Uuid) {
    // Read messages — brief lock
    let messages = {
        let mem = state.memory.lock().await;
        mem.working.messages(session_id).to_vec()
    };

    // Only extract if there are error→correction patterns
    if !detect_lesson_patterns(&messages) {
        return;
    }

    info!(session = %session_id, "detected error→correction pattern, extracting lessons");

    let lessons = extract_lessons_via_llm(state, &messages).await;
    if lessons.is_empty() {
        return;
    }

    info!(session = %session_id, count = lessons.len(), "extracted lessons from conversation");

    // Persist each lesson as a semantic fact
    let mut mem = state.memory.lock().await;
    for (key, lesson) in &lessons {
        let fact = claw_memory::semantic::Fact {
            id: Uuid::new_v4(),
            category: "learned_lessons".to_string(),
            key: key.clone(),
            value: lesson.clone(),
            confidence: 0.9,
            source: Some(format!("session:{session_id}")),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        mem.semantic.upsert(fact);
        if let Err(e) = mem.persist_fact("learned_lessons", key, lesson) {
            warn!(error = %e, key = key, "failed to persist lesson to SQLite");
        }
    }
    drop(mem);

    // Generate embeddings for the lessons if embedder is available
    if let Some(ref embedder) = state.embedder {
        let texts: Vec<String> = lessons
            .iter()
            .map(|(key, lesson)| format!("learned_lessons {key} {lesson}"))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        if let Ok(embeddings) = embedder.embed(&text_refs).await {
            let mem = state.memory.lock().await;
            for (i, (key, _lesson)) in lessons.iter().enumerate() {
                if let Some(emb) = embeddings.get(i) {
                    // Re-persist with embedding
                    if let Some(fact) = mem.semantic.get("learned_lessons", key) {
                        let _ = mem.persist_fact_with_embedding(
                            "learned_lessons",
                            key,
                            &fact.value,
                            Some(emb),
                        );
                    }
                }
            }
        }
    }

    // Broadcast lessons to mesh peers
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() && mesh.peer_count() > 0 {
            for (key, lesson) in &lessons {
                let sync_msg = MeshMessage::SyncDelta {
                    peer_id: mesh.peer_id().to_string(),
                    delta_type: "fact".to_string(),
                    data: serde_json::json!({
                        "category": "learned_lessons",
                        "key": key,
                        "value": lesson,
                        "confidence": 0.9,
                    }),
                };
                let _ = mesh.broadcast(&sync_msg).await;
            }
        }
    }
}
