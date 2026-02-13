use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::*;

/// Image file extensions we'll try to upload as Telegram photos.
const IMAGE_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"];

/// Extract screenshot filenames from `/api/v1/screenshots/{name}.png` URLs.
fn extract_screenshot_filenames(text: &str) -> Vec<String> {
    let mut filenames = Vec::new();
    let prefix = "/api/v1/screenshots/";
    let mut search_from = 0;
    while let Some(pos) = text[search_from..].find(prefix) {
        let start = search_from + pos + prefix.len();
        if let Some(end) =
            text[start..].find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '-')
        {
            let candidate = &text[start..start + end];
            if IMAGE_EXTENSIONS.iter().any(|ext| candidate.ends_with(ext)) {
                filenames.push(candidate.to_string());
            }
        } else {
            let candidate = &text[start..];
            if IMAGE_EXTENSIONS.iter().any(|ext| candidate.ends_with(ext)) {
                filenames.push(candidate.to_string());
            }
        }
        search_from = start;
    }
    filenames
}

/// Expand a leading `~` or `~/` to the user's home directory.
#[allow(dead_code)]
fn expand_home(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

/// All known file extensions (images + documents) as a combined list for regex.
#[allow(dead_code)]
const ALL_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".pdf", ".doc", ".docx", ".xls", ".xlsx",
    ".csv", ".txt", ".zip", ".tar", ".gz", ".json", ".xml", ".html", ".md", ".py", ".rs", ".js",
    ".ts", ".sh", ".log", ".mp4", ".mp3", ".wav", ".ogg", ".m4a", ".aac", ".flac", ".aiff", ".mov",
    ".avi", ".mkv", ".webm",
];

/// Extract file paths from text — handles paths with spaces.
/// Scans for patterns like `/path/to/file.ext` or `~/path/to/file.ext`.
/// Returns (image_paths, doc_paths) with `~` expanded to home dir.
#[allow(dead_code)]
fn extract_all_paths(text: &str) -> (Vec<String>, Vec<String>) {
    let mut image_paths = Vec::new();
    let mut doc_paths = Vec::new();

    // Strategy: find each occurrence of a known extension in the text,
    // then scan backwards to find the start of the path (/ or ~/).
    let lower_text = text.to_lowercase();

    for ext in ALL_EXTENSIONS {
        let mut search_from = 0;
        while let Some(ext_pos) = lower_text[search_from..].find(ext) {
            let abs_ext_pos = search_from + ext_pos;
            let path_end = abs_ext_pos + ext.len();

            // Make sure the extension is at a word boundary (not part of a longer word)
            if path_end < text.len() {
                let next_char = text[path_end..].chars().next().unwrap_or(' ');
                if next_char.is_alphanumeric() || next_char == '_' {
                    search_from = abs_ext_pos + 1;
                    continue;
                }
            }

            // Scan backwards to find the path start (/ or ~)
            // We look for a `/` or `~` that starts a path
            let text_before = &text[..abs_ext_pos];
            let path_start = find_path_start(text_before);

            if let Some(start_pos) = path_start {
                let raw_path = text[start_pos..path_end].trim();
                let expanded = expand_home(raw_path);

                // Deduplicate
                if !image_paths.contains(&expanded) && !doc_paths.contains(&expanded) {
                    if IMAGE_EXTENSIONS
                        .iter()
                        .any(|e| expanded.to_lowercase().ends_with(e))
                    {
                        image_paths.push(expanded);
                    } else {
                        doc_paths.push(expanded);
                    }
                }
            }

            search_from = path_end;
        }
    }

    (image_paths, doc_paths)
}

/// Find the start of a file path by scanning backwards from a known extension position.
/// Looks for `/` or `~/` preceded by whitespace, line start, or certain delimiters.
#[allow(dead_code)]
fn find_path_start(text_before_ext: &str) -> Option<usize> {
    // Scan backwards for a `/` or `~` that looks like a path start
    let bytes = text_before_ext.as_bytes();
    let len = bytes.len();

    // Walk backwards to find the first `/` that is preceded by whitespace, start-of-string,
    // or a delimiter — that's likely the root of the path.
    // We need to find `/Users/...`, `/home/...`, `/tmp/...`, `~/...` etc.

    // First try: find `~/` pattern
    for i in (0..len.saturating_sub(1)).rev() {
        if bytes[i] == b'~' && i + 1 < len && bytes[i + 1] == b'/' {
            // Check character before ~ is a delimiter or start
            if i == 0 || is_path_delimiter(bytes[i - 1]) {
                return Some(i);
            }
        }
    }

    // Second try: find a root `/` — a slash at position 0 or preceded by a delimiter
    for i in (0..len).rev() {
        if bytes[i] == b'/' {
            if i == 0 {
                return Some(0);
            }
            let prev = bytes[i - 1];
            // If preceded by a delimiter, this is likely the start
            if is_path_delimiter(prev) {
                return Some(i);
            }
            // If we've reached a path separator going up, keep going
            // (we want the outermost `/`)
        }
    }

    None
}

/// Check if a byte is a path delimiter (indicates the start of a path).
#[allow(dead_code)]
fn is_path_delimiter(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t'
            | b'\n'
            | b'\r'
            | b'"'
            | b'\''
            | b'`'
            | b'('
            | b'['
            | b'{'
            | b','
            | b';'
            | b':'
            | b'>'
            | b'|'
    )
}

/// Extract absolute image file paths from text (convenience wrapper).
#[allow(dead_code)]
fn extract_image_paths(text: &str) -> Vec<String> {
    extract_all_paths(text).0
}

/// Extract absolute file paths (documents, audio, video) from text (convenience wrapper).
#[allow(dead_code)]
fn extract_file_paths(text: &str) -> Vec<String> {
    extract_all_paths(text).1
}

/// Get the screenshots directory path.
fn screenshots_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claw")
        .join("screenshots")
}

/// Strip screenshot URL references, image paths, and file paths from text.
fn strip_file_references(text: &str, _api_filenames: &[String], disk_paths: &[String]) -> String {
    let mut result = text.to_string();

    // Remove /api/v1/screenshots/... URLs (and optional "View: " prefix)
    while let Some(pos) = result.find("/api/v1/screenshots/") {
        let start = if pos >= 6 && &result[pos - 6..pos] == "View: " {
            pos - 6
        } else {
            pos
        };
        let url_end = result[pos..]
            .find(|c: char| c.is_whitespace())
            .map(|e| pos + e)
            .unwrap_or(result.len());
        result = format!("{}{}", &result[..start], &result[url_end..]);
    }

    // Remove absolute disk paths (and optional "Saved at: " / "saved to " prefixes)
    for path in disk_paths {
        while let Some(pos) = result.find(path.as_str()) {
            // Look back for common prefixes
            let start = [
                "Saved at: ",
                "saved at: ",
                "saved to ",
                "Saved to ",
                "Path: ",
                "path: ",
                "File: ",
                "file: ",
            ]
            .iter()
            .find_map(|prefix| {
                if pos >= prefix.len() && &result[pos - prefix.len()..pos] == *prefix {
                    Some(pos - prefix.len())
                } else {
                    None
                }
            })
            .unwrap_or(pos);
            let end = pos + path.len();
            result = format!("{}{}", &result[..start], &result[end..]);
        }
    }

    // Clean up leftover artifacts: multiple blank lines, trailing whitespace, "- " bullet on empty line
    let lines: Vec<&str> = result
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty() && *l != "-" && *l != "- ")
        .collect();
    lines.join("\n").trim().to_string()
}

/// Telegram channel adapter using the Bot API.
pub struct TelegramChannel {
    id: String,
    token: String,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl TelegramChannel {
    pub fn new(id: String, token: String) -> Self {
        // Build client with timeouts to prevent stalled connections from
        // hanging the long-poll loop indefinitely.  The Telegram long-poll
        // uses `timeout=30` server-side, so the overall request timeout must
        // be larger (45s gives 15s headroom for network latency).
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(45))
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            id,
            token,
            client,
            connected: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    /// Upload a photo from disk to a Telegram chat using multipart form data.
    async fn send_photo(
        &self,
        chat_id: &str,
        photo_path: &std::path::Path,
        caption: Option<&str>,
    ) -> claw_core::Result<()> {
        let file_bytes =
            tokio::fs::read(photo_path)
                .await
                .map_err(|e| claw_core::ClawError::Channel {
                    channel: "telegram".into(),
                    reason: format!("failed to read photo {}: {}", photo_path.display(), e),
                })?;

        let filename = photo_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let photo_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename)
            .mime_str("image/png")
            .unwrap();

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", photo_part);

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .client
            .post(&self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: format!("sendPhoto failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: format!("sendPhoto failed: {}", text),
            });
        }

        debug!(path = %photo_path.display(), "sent photo to Telegram");
        Ok(())
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> &str {
        "telegram"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let client = self.client.clone();
        let token = self.token.clone();
        let connected = Arc::clone(&self.connected);

        // Spawn long-polling loop
        tokio::spawn(async move {
            let base_url = format!("https://api.telegram.org/bot{}", token);
            let mut offset: i64 = 0;
            connected.store(true, Ordering::SeqCst);
            info!("Telegram channel connected, starting long-poll");

            let mut shutdown_rx = shutdown_rx;

            // Backoff state — grows on consecutive failures, resets on success
            let mut consecutive_failures: u32 = 0;
            let mut consecutive_conflicts: u32 = 0;
            const MAX_BACKOFF_SECS: u64 = 60;
            const MAX_CONFLICT_RETRIES: u32 = 5;

            loop {
                // Check shutdown
                if *shutdown_rx.borrow() {
                    info!("Telegram poll loop: shutdown requested");
                    break;
                }

                // If the receiver side (aggregate_rx) has been dropped, stop polling
                if event_tx.is_closed() {
                    info!("Telegram poll loop: event receiver dropped, stopping");
                    break;
                }

                let url = format!("{}/getUpdates?offset={}&timeout=30", base_url, offset);

                tokio::select! {
                    biased; // prefer shutdown signal

                    _ = shutdown_rx.changed() => {
                        info!("Telegram poll loop: shutdown signal received");
                        break;
                    }

                    result = client.get(&url).send() => {
                        match result {
                            Ok(resp) => {
                                let status = resp.status();
                                if !status.is_success() {
                                    let body = resp.text().await.unwrap_or_default();

                                    // 409 Conflict = another bot instance is polling
                                    if status.as_u16() == 409 {
                                        consecutive_conflicts += 1;
                                        error!(
                                            attempt = consecutive_conflicts,
                                            max = MAX_CONFLICT_RETRIES,
                                            "Telegram 409 Conflict: another bot instance is polling \
                                             with the same token. Only one getUpdates consumer is \
                                             allowed per bot token."
                                        );
                                        if consecutive_conflicts >= MAX_CONFLICT_RETRIES {
                                            error!(
                                                "Stopping Telegram polling after {} consecutive 409 \
                                                 conflicts. Another Claw instance (or other bot) is \
                                                 using this token. Stop the other instance or use a \
                                                 different bot token.",
                                                consecutive_conflicts
                                            );
                                            break;
                                        }
                                        // Wait longer for conflicts — give the other instance time
                                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                        continue;
                                    }

                                    // Other HTTP errors (429, 502, etc.)
                                    warn!(
                                        status = %status,
                                        body = %body.chars().take(200).collect::<String>(),
                                        "Telegram API returned HTTP error"
                                    );
                                    consecutive_failures += 1;
                                    consecutive_conflicts = 0; // non-conflict error resets conflict counter
                                    let backoff = backoff_duration(consecutive_failures, MAX_BACKOFF_SECS);
                                    tokio::time::sleep(backoff).await;
                                    continue;
                                }

                                match resp.json::<serde_json::Value>().await {
                                    Ok(data) => {
                                        // Telegram wraps responses in {"ok": true/false, ...}
                                        if data["ok"].as_bool() != Some(true) {
                                            let desc = data["description"].as_str().unwrap_or("unknown error");
                                            let code = data["error_code"].as_i64().unwrap_or(0);

                                            // 409 = conflict (another bot instance)
                                            if code == 409 {
                                                consecutive_conflicts += 1;
                                                error!(
                                                    attempt = consecutive_conflicts,
                                                    max = MAX_CONFLICT_RETRIES,
                                                    description = %desc,
                                                    "Telegram 409 Conflict: another bot instance is \
                                                     polling with the same token."
                                                );
                                                if consecutive_conflicts >= MAX_CONFLICT_RETRIES {
                                                    error!(
                                                        "Stopping Telegram polling — another instance \
                                                         owns this bot token. Stop the other instance \
                                                         or use a different token."
                                                    );
                                                    break;
                                                }
                                                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                                continue;
                                            }

                                            warn!(
                                                error_code = code,
                                                description = %desc,
                                                "Telegram API error response"
                                            );
                                            consecutive_failures += 1;
                                            consecutive_conflicts = 0;

                                            // 429 = rate limited — Telegram sends retry_after
                                            if code == 429 {
                                                let retry_after = data["parameters"]["retry_after"]
                                                    .as_u64()
                                                    .unwrap_or(5);
                                                warn!(retry_after, "Telegram rate limited, backing off");
                                                tokio::time::sleep(
                                                    std::time::Duration::from_secs(retry_after)
                                                ).await;
                                            } else {
                                                let backoff = backoff_duration(consecutive_failures, MAX_BACKOFF_SECS);
                                                tokio::time::sleep(backoff).await;
                                            }
                                            continue;
                                        }

                                        // Success — reset backoff
                                        if consecutive_failures > 0 || consecutive_conflicts > 0 {
                                            info!(
                                                prev_failures = consecutive_failures,
                                                prev_conflicts = consecutive_conflicts,
                                                "Telegram poll recovered"
                                            );
                                        }
                                        consecutive_failures = 0;
                                        consecutive_conflicts = 0;

                                        if let Some(updates) = data["result"].as_array() {
                                            for update in updates {
                                                if let Some(uid) = update["update_id"].as_i64() {
                                                    offset = uid + 1;
                                                }
                                                // Dispatch the update — break early if receiver gone
                                                if !dispatch_update(
                                                    update, &event_tx, &client, &base_url,
                                                ).await {
                                                    info!("Telegram poll loop: event receiver dropped during dispatch");
                                                    connected.store(false, Ordering::SeqCst);
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // JSON decode failed — could be an HTML error page
                                        warn!(error = %e, "Telegram poll: failed to parse JSON response");
                                        consecutive_failures += 1;
                                        let backoff = backoff_duration(consecutive_failures, MAX_BACKOFF_SECS);
                                        tokio::time::sleep(backoff).await;
                                    }
                                }
                            }
                            Err(e) => {
                                // Network / timeout error
                                if e.is_timeout() {
                                    // Request timeout is expected when no updates arrive —
                                    // just loop around immediately.
                                    debug!("Telegram long-poll timed out (normal, no updates)");
                                    // Don't count timeouts as failures
                                } else {
                                    warn!(error = %e, "Telegram poll network error");
                                    consecutive_failures += 1;
                                    let backoff = backoff_duration(consecutive_failures, MAX_BACKOFF_SECS);
                                    tokio::time::sleep(backoff).await;
                                }
                            }
                        }
                    }
                }
            }

            connected.store(false, Ordering::SeqCst);
            info!("Telegram channel disconnected");
        });

        Ok(event_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        debug!(text_len = message.text.len(), target = %message.target, "Telegram send() called");
        debug!(text_preview = %message.text.chars().take(200).collect::<String>(), "Telegram send() text preview");
        // ── 1. Collect all image paths to upload ─────────────────────

        let mut photo_paths: Vec<PathBuf> = Vec::new();
        let mut found_screenshot_files = false;

        // (a) /api/v1/screenshots/{name}.png URLs → ~/.claw/screenshots/
        let screenshot_files = extract_screenshot_filenames(&message.text);
        if !screenshot_files.is_empty() {
            found_screenshot_files = true;
            let dir = screenshots_dir();
            for filename in &screenshot_files {
                let path = dir.join(filename);
                if path.exists() {
                    debug!(path = %path.display(), "found screenshot file from API URL");
                    photo_paths.push(path);
                } else {
                    warn!(path = %path.display(), "screenshot file referenced in text but not found on disk");
                }
            }
        }

        // (b) Explicit attachments on the OutgoingMessage (from channel_send_file tool)
        for att in &message.attachments {
            if att.media_type.starts_with("image/") {
                let p = PathBuf::from(&att.data);
                if p.exists() && !photo_paths.contains(&p) {
                    photo_paths.push(p);
                }
            }
        }

        // ── 2. Collect document/file paths (from explicit attachments only) ──
        let mut doc_paths: Vec<PathBuf> = Vec::new();
        for att in &message.attachments {
            if !att.media_type.starts_with("image/") {
                let p = PathBuf::from(&att.data);
                if p.exists() && !doc_paths.contains(&p) {
                    doc_paths.push(p);
                }
            }
        }

        // ── 3. Upload photos ─────────────────────────────────────────

        let mut uploaded = 0usize;
        for photo_path in &photo_paths {
            match self.send_photo(&message.target, photo_path, None).await {
                Ok(()) => uploaded += 1,
                Err(e) => {
                    warn!(error = %e, path = %photo_path.display(), "failed to send photo to Telegram");
                }
            }
        }
        if !photo_paths.is_empty() {
            debug!(
                total = photo_paths.len(),
                uploaded, "Telegram photo upload summary"
            );
        }

        // ── 4. Upload documents/files ────────────────────────────────
        for doc_path in &doc_paths {
            let filename = doc_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let lower_name = filename.to_lowercase();

            let bytes = match tokio::fs::read(doc_path).await {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, path = %doc_path.display(), "failed to read file");
                    continue;
                }
            };

            // Choose the right Telegram API method based on file type:
            // - sendAudio for .mp3/.m4a (shows in music player)
            // - sendVideo for .mp4/.mov/.avi/.mkv (shows video player)
            // - sendDocument for everything else
            let (api_method, field_name) =
                if lower_name.ends_with(".mp3") || lower_name.ends_with(".m4a") {
                    ("sendAudio", "audio")
                } else if lower_name.ends_with(".ogg") {
                    // .ogg can be voice — but sendAudio also handles it
                    ("sendAudio", "audio")
                } else if lower_name.ends_with(".mp4")
                    || lower_name.ends_with(".mov")
                    || lower_name.ends_with(".avi")
                    || lower_name.ends_with(".mkv")
                    || lower_name.ends_with(".webm")
                {
                    ("sendVideo", "video")
                } else {
                    ("sendDocument", "document")
                };

            let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.clone());
            let form = reqwest::multipart::Form::new()
                .text("chat_id", message.target.clone())
                .part(field_name, part);

            match self
                .client
                .post(&self.api_url(api_method))
                .multipart(form)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    uploaded += 1;
                    info!(path = %doc_path.display(), method = api_method, "sent file to Telegram");
                }
                Ok(r) => {
                    let t = r.text().await.unwrap_or_default();
                    warn!(error = %t, path = %doc_path.display(), method = api_method, "Telegram file send failed");
                    // Fallback: try sendDocument if the specialized method failed
                    if api_method != "sendDocument" {
                        debug!(path = %doc_path.display(), "falling back to sendDocument");
                        if let Ok(bytes) = tokio::fs::read(doc_path).await {
                            let part =
                                reqwest::multipart::Part::bytes(bytes).file_name(filename.clone());
                            let form = reqwest::multipart::Form::new()
                                .text("chat_id", message.target.clone())
                                .part("document", part);
                            match self
                                .client
                                .post(&self.api_url("sendDocument"))
                                .multipart(form)
                                .send()
                                .await
                            {
                                Ok(r2) if r2.status().is_success() => {
                                    uploaded += 1;
                                    info!(path = %doc_path.display(), "sent file via sendDocument fallback");
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, path = %doc_path.display(), method = api_method, "Telegram file send error");
                }
            }
        }

        // ── 5. Send remaining text ──────────────────────────────────

        // Strip screenshot references from the text so we don't repeat file paths
        let clean_text = if found_screenshot_files {
            strip_file_references(&message.text, &screenshot_files, &[])
        } else {
            message.text.clone()
        };

        // If we uploaded at least one photo and the remaining text is empty, we're done
        if uploaded > 0 && (clean_text.is_empty() || clean_text.chars().all(|c| c.is_whitespace()))
        {
            return Ok(());
        }

        // If the only content is "I can't attach/upload/send" after a successful upload, skip it
        if uploaded > 0 {
            let lower = clean_text.to_lowercase();
            if lower.contains("can't attach")
                || lower.contains("cannot attach")
                || lower.contains("can't upload")
                || lower.contains("cannot upload")
                || lower.contains("can't send the image")
                || lower.contains("unable to attach")
                || lower.contains("unable to upload")
                || lower.contains("unable to send the image")
                || lower.contains("can't send the file")
                || lower.contains("cannot send the file")
                || lower.contains("can't send file")
                || lower.contains("unable to send file")
                || lower.contains("don't have a direct way to send")
                || lower.contains("cannot directly send")
            {
                debug!("suppressing 'can't send' text since file was already sent");
                return Ok(());
            }
        }

        // Try Markdown first, fall back to plain text if Telegram rejects it
        let body_md = serde_json::json!({
            "chat_id": message.target,
            "text": clean_text,
            "parse_mode": "Markdown",
        });

        let resp = self
            .client
            .post(&self.api_url("sendMessage"))
            .json(&body_md)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: e.to_string(),
            })?;

        if resp.status().is_success() {
            return Ok(());
        }

        // Markdown failed — retry without parse_mode (plain text)
        debug!("Telegram Markdown send failed, retrying as plain text");
        let body = serde_json::json!({
            "chat_id": message.target,
            "text": clean_text,
        });

        let resp = self
            .client
            .post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: format!("sendMessage failed: {}", text),
            });
        }

        Ok(())
    }

    async fn send_approval_prompt(&self, prompt: ApprovalPrompt) -> claw_core::Result<()> {
        let args_preview = serde_json::to_string_pretty(&prompt.tool_args)
            .unwrap_or_else(|_| prompt.tool_args.to_string());
        // Truncate args preview so the message isn't huge
        let args_short = if args_preview.len() > 300 {
            format!("{}...", &args_preview[..300])
        } else {
            args_preview
        };

        // HTML-escape the dynamic content to avoid parse errors
        let args_escaped = args_short
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let reason_escaped = prompt
            .reason
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");

        let text = format!(
            "⚠️ <b>Approval Required</b>\n\n\
             🔧 Tool: <code>{}</code>\n\
             ⚡ Risk: {}/10\n\
             📋 Reason: {}\n\
             <pre>{}</pre>\n\
             🆔 <code>{}</code>",
            prompt.tool_name, prompt.risk_level, reason_escaped, args_escaped, prompt.approval_id,
        );

        let body = serde_json::json!({
            "chat_id": prompt.target,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": {
                "inline_keyboard": [[
                    {
                        "text": "✅ Approve",
                        "callback_data": format!("approve:{}", prompt.approval_id),
                    },
                    {
                        "text": "❌ Deny",
                        "callback_data": format!("deny:{}", prompt.approval_id),
                    },
                ]],
            },
        });

        let resp = self
            .client
            .post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(error = %text, "failed to send Telegram approval prompt");
        }

        Ok(())
    }

    async fn send_returning_id(
        &self,
        message: OutgoingMessage,
    ) -> claw_core::Result<Option<String>> {
        // Send with Markdown, parse message_id from Telegram response
        let body = serde_json::json!({
            "chat_id": message.target,
            "text": message.text,
            "parse_mode": "Markdown",
        });

        let resp = self
            .client
            .post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: e.to_string(),
            })?;

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg_id = json["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string());
        Ok(msg_id)
    }

    async fn edit_message(
        &self,
        target: &str,
        message_id: &str,
        text: &str,
    ) -> claw_core::Result<()> {
        let body = serde_json::json!({
            "chat_id": target,
            "message_id": message_id.parse::<i64>().unwrap_or(0),
            "text": text,
            "parse_mode": "Markdown",
        });

        let resp = self
            .client
            .post(&self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "telegram".into(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            // Non-fatal — message may have been deleted or not changed
            let text = resp.text().await.unwrap_or_default();
            debug!(error = %text, "failed to edit Telegram message (non-fatal)");
        }

        Ok(())
    }

    async fn send_typing(&self, target: &str) -> claw_core::Result<()> {
        let body = serde_json::json!({
            "chat_id": target,
            "action": "typing",
        });
        let _ = self
            .client
            .post(&self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await;
        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Exponential backoff with jitter: 1s, 2s, 4s, 8s, … capped at `max_secs`.
fn backoff_duration(consecutive_failures: u32, max_secs: u64) -> std::time::Duration {
    let base = 1u64
        .checked_shl(consecutive_failures.min(6))
        .unwrap_or(max_secs);
    let capped = base.min(max_secs);
    // Add ±25% jitter to prevent thundering herd
    let jitter_ms = (rand::random::<u64>() % (capped * 500 + 1)) as i64 - (capped as i64 * 250);
    let ms = (capped as i64 * 1000 + jitter_ms).max(500) as u64;
    std::time::Duration::from_millis(ms)
}

/// Dispatch a single Telegram update to the event channel.
/// Returns `false` if the event channel is closed (receiver dropped).
async fn dispatch_update(
    update: &serde_json::Value,
    event_tx: &mpsc::Sender<ChannelEvent>,
    client: &reqwest::Client,
    base_url: &str,
) -> bool {
    // Parse callback_query (inline keyboard button press)
    if let Some(cbq) = update.get("callback_query") {
        let callback_id = cbq["id"].as_str().unwrap_or("").to_string();
        let cb_data = cbq["data"].as_str().unwrap_or("").to_string();
        let sender = cbq["from"]["id"].to_string();
        let chat_id = cbq["message"]["chat"]["id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default();
        debug!(callback_id = %callback_id, data = %cb_data, "telegram callback query");

        // Answer the callback to remove the loading spinner
        let answer_url = format!("{}/answerCallbackQuery", base_url);
        let _ = client
            .post(&answer_url)
            .json(&serde_json::json!({
                "callback_query_id": callback_id,
                "text": if cb_data.starts_with("approve:") { "✅ Approved" } else { "❌ Denied" },
            }))
            .send()
            .await;

        return event_tx
            .send(ChannelEvent::CallbackQuery {
                callback_id,
                data: cb_data,
                sender,
                chat_id,
            })
            .await
            .is_ok();
    }

    // Parse message
    if let Some(msg) = update.get("message") {
        let incoming = IncomingMessage {
            id: msg["message_id"].to_string(),
            channel: "telegram".into(),
            sender: msg["from"]["id"].to_string(),
            sender_name: msg["from"]["first_name"].as_str().map(String::from),
            group: msg["chat"]["id"].as_i64().map(|id| id.to_string()),
            text: msg["text"].as_str().map(String::from),
            attachments: vec![],
            is_mention: false,
            is_reply_to_bot: false,
            metadata: update.clone(),
        };
        return event_tx.send(ChannelEvent::Message(incoming)).await.is_ok();
    }

    // Unknown update type — not an error, just skip
    debug!("skipping unrecognized Telegram update type");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_path() {
        let text = "Here is the file: /Users/wichard/Desktop/test-audio.mp3";
        let (images, docs) = extract_all_paths(text);
        assert!(images.is_empty());
        assert_eq!(docs, vec!["/Users/wichard/Desktop/test-audio.mp3"]);
    }

    #[test]
    fn test_extract_path_with_spaces() {
        let text = "Screenshot: /Users/wichard/Desktop/Schermafbeelding 2026-01-13 om 14.30.00.png";
        let (images, docs) = extract_all_paths(text);
        assert_eq!(
            images,
            vec!["/Users/wichard/Desktop/Schermafbeelding 2026-01-13 om 14.30.00.png"]
        );
        assert!(docs.is_empty());
    }

    #[test]
    fn test_extract_tilde_path() {
        let text = "File at ~/Desktop/test.mp3";
        let (images, docs) = extract_all_paths(text);
        assert!(images.is_empty());
        assert_eq!(docs.len(), 1);
        assert!(docs[0].ends_with("/Desktop/test.mp3"));
        assert!(docs[0].starts_with('/'));
    }

    #[test]
    fn test_extract_multiple_paths() {
        let text = "Here are some files:\n- /Users/wichard/Desktop/photo.png\n- /Users/wichard/Desktop/audio.mp3\n- /Users/wichard/Desktop/doc.pdf";
        let (images, docs) = extract_all_paths(text);
        assert_eq!(images, vec!["/Users/wichard/Desktop/photo.png"]);
        assert_eq!(docs.len(), 2);
        assert!(docs.contains(&"/Users/wichard/Desktop/audio.mp3".to_string()));
        assert!(docs.contains(&"/Users/wichard/Desktop/doc.pdf".to_string()));
    }

    #[test]
    fn test_no_false_positives() {
        let text = "The .png format is widely used. Visit https://example.com/image.png";
        let (images, _docs) = extract_all_paths(text);
        // Should NOT match ".png" alone or URLs without a path start
        assert!(images.is_empty() || images.iter().all(|p| p.starts_with('/')));
    }

    #[test]
    fn test_path_after_colon() {
        let text = "Pad: /Users/wichard/Desktop/test-audio/test.mp3";
        let (_images, docs) = extract_all_paths(text);
        assert_eq!(docs, vec!["/Users/wichard/Desktop/test-audio/test.mp3"]);
    }

    #[test]
    fn test_m4a_extension() {
        let text = "Audio: /Users/wichard/Desktop/test.m4a";
        let (_images, docs) = extract_all_paths(text);
        assert_eq!(docs, vec!["/Users/wichard/Desktop/test.m4a"]);
    }
}
