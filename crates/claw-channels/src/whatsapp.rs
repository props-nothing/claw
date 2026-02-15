use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::*;

/// Image file extensions we'll try to upload as WhatsApp photos.
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

/// All known file extensions (images + documents) as a combined list.
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

    let lower_text = text.to_lowercase();

    for ext in ALL_EXTENSIONS {
        let mut search_from = 0;
        while let Some(ext_pos) = lower_text[search_from..].find(ext) {
            let abs_ext_pos = search_from + ext_pos;
            let path_end = abs_ext_pos + ext.len();

            // Make sure extension is at a word boundary
            if path_end < text.len() {
                let next_char = text[path_end..].chars().next().unwrap_or(' ');
                if next_char.is_alphanumeric() || next_char == '_' {
                    search_from = abs_ext_pos + 1;
                    continue;
                }
            }

            // Scan backwards to find path start
            let text_before = &text[..abs_ext_pos];
            let path_start = find_path_start(text_before);

            if let Some(start_pos) = path_start {
                let raw_path = text[start_pos..path_end].trim();
                let expanded = expand_home(raw_path);

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
#[allow(dead_code)]
fn find_path_start(text_before_ext: &str) -> Option<usize> {
    let bytes = text_before_ext.as_bytes();
    let len = bytes.len();

    // First try: find `~/` pattern
    for i in (0..len.saturating_sub(1)).rev() {
        if bytes[i] == b'~'
            && i + 1 < len
            && bytes[i + 1] == b'/'
            && (i == 0 || is_path_delimiter(bytes[i - 1]))
        {
            return Some(i);
        }
    }

    // Second try: find a root `/` preceded by a delimiter or start-of-string
    for i in (0..len).rev() {
        if bytes[i] == b'/' {
            if i == 0 {
                return Some(0);
            }
            let prev = bytes[i - 1];
            if is_path_delimiter(prev) {
                return Some(i);
            }
        }
    }

    None
}

/// Check if a byte is a path delimiter.
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

/// Extract absolute image file paths from text.
#[allow(dead_code)]
fn extract_image_paths(text: &str) -> Vec<String> {
    extract_all_paths(text).0
}

/// Extract absolute file paths (documents) from text.
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

/// Strip image and file path references from text.
fn strip_file_references(text: &str, screenshot_files: &[String], disk_paths: &[String]) -> String {
    let mut result = text.to_string();

    // Remove /api/v1/screenshots/... URLs
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

    // Remove absolute disk paths
    for path in disk_paths {
        while let Some(pos) = result.find(path.as_str()) {
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

    // Clean up extra whitespace/newlines
    let _ = screenshot_files; // used above in URL matching
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}

/// WhatsApp channel adapter using a Node.js Baileys bridge subprocess.
///
/// ## Architecture
///
/// WhatsApp Web uses a proprietary binary protocol implemented by the
/// `@whiskeysockets/baileys` Node.js library (same library OpenClaw uses).
/// Rather than reimplementing this complex protocol in Rust, we spawn a small
/// Node.js bridge process that:
///
/// 1. Connects to WhatsApp via Baileys
/// 2. Handles QR code generation for device linking
/// 3. Communicates with claw via JSON messages over stdin/stdout
///
/// The bridge script is bundled at `~/.claw/bridges/whatsapp-bridge.js` and
/// is auto-installed on first use.
///
/// ## Setup
///
/// 1. Ensure Node.js ≥ 18 is installed
/// 2. Run `claw channels login whatsapp` — this installs the bridge and shows QR
/// 3. Scan the QR code with WhatsApp on your phone
/// 4. Done! Session persists across restarts.
///
/// ## DM Policy
///
/// - `pairing` (default): unknown senders receive a short code; messages are
///   NOT processed until approved via `claw channels approve whatsapp <CODE>`.
/// - `allowlist`: only numbers in `allow_from` can message.
/// - `open`: anyone can message.
/// - `disabled`: ignore all DMs.
pub struct WhatsAppChannel {
    id: String,
    /// Phone number associated with this WhatsApp account (after linking).
    phone: Option<String>,
    /// Directory storing Baileys auth state.
    auth_dir: PathBuf,
    /// DM access policy.
    dm_policy: DmPolicy,
    /// Allowed sender numbers (E.164 format).
    allow_from: Vec<String>,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

/// DM access policy — mirrors OpenClaw's pairing system.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DmPolicy {
    /// Unknown senders get a pairing code; owner must approve.
    #[default]
    Pairing,
    /// Only numbers in `allow_from` can chat.
    Allowlist,
    /// Anyone can message (public).
    Open,
    /// WhatsApp DMs are disabled entirely.
    Disabled,
}

impl std::fmt::Display for DmPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DmPolicy::Pairing => write!(f, "pairing"),
            DmPolicy::Allowlist => write!(f, "allowlist"),
            DmPolicy::Open => write!(f, "open"),
            DmPolicy::Disabled => write!(f, "disabled"),
        }
    }
}

impl std::str::FromStr for DmPolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pairing" => Ok(DmPolicy::Pairing),
            "allowlist" => Ok(DmPolicy::Allowlist),
            "open" => Ok(DmPolicy::Open),
            "disabled" => Ok(DmPolicy::Disabled),
            _ => Err(format!(
                "unknown DM policy: '{s}' (valid: pairing, allowlist, open, disabled)"
            )),
        }
    }
}

/// Active pairing request from an unknown sender.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PairingRequest {
    pub code: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub channel: String,
    pub created_at: String,
    pub expires_at: String,
}

/// Result of a QR login attempt.
#[derive(Debug, Clone)]
pub struct QrLoginResult {
    /// Data URL containing the QR code image (data:image/png;base64,...).
    pub qr_data_url: Option<String>,
    /// Raw QR string for terminal rendering.
    pub qr_raw: Option<String>,
    /// Status message.
    pub message: String,
    /// Whether the device is now connected.
    pub connected: bool,
}

impl WhatsAppChannel {
    pub fn new(id: String, auth_dir: Option<PathBuf>) -> Self {
        let default_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".claw")
            .join("credentials")
            .join("whatsapp");

        let auth_dir = auth_dir.unwrap_or(default_dir);

        Self {
            id,
            phone: None,
            auth_dir,
            dm_policy: DmPolicy::Pairing,
            allow_from: Vec::new(),
            client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
        }
    }

    /// Set the DM access policy.
    pub fn with_dm_policy(mut self, policy: DmPolicy) -> Self {
        self.dm_policy = policy;
        self
    }

    /// Set the allowed sender list (E.164 phone numbers).
    pub fn with_allow_from(mut self, numbers: Vec<String>) -> Self {
        self.allow_from = numbers;
        self
    }

    /// Check if WhatsApp auth credentials exist on disk (i.e. previously linked).
    pub fn is_linked(&self) -> bool {
        self.auth_dir.join("creds.json").exists()
    }

    /// Get the auth directory path.
    pub fn auth_dir(&self) -> &PathBuf {
        &self.auth_dir
    }

    /// Get pending pairing requests from the store.
    pub fn load_pairing_requests(&self) -> Vec<PairingRequest> {
        let path = self.auth_dir.join("pairing.json");
        if !path.exists() {
            return Vec::new();
        }
        match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Save pairing requests to disk.
    fn save_pairing_requests(&self, requests: &[PairingRequest]) -> claw_core::Result<()> {
        let _ = std::fs::create_dir_all(&self.auth_dir);
        let path = self.auth_dir.join("pairing.json");
        let data =
            serde_json::to_string_pretty(requests).map_err(|e| claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: format!("failed to serialize pairing requests: {e}"),
            })?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Approve a pairing request by code. Returns the approved sender.
    pub fn approve_pairing(&self, code: &str) -> claw_core::Result<String> {
        let mut requests = self.load_pairing_requests();
        let pos = requests
            .iter()
            .position(|r| r.code == code)
            .ok_or_else(|| claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: format!("pairing code '{code}' not found"),
            })?;
        let approved = requests.remove(pos);
        self.save_pairing_requests(&requests)?;

        // Add to allowlist store
        let allowlist_path = self.auth_dir.join("allow-from.json");
        let mut allowlist: Vec<String> = if allowlist_path.exists() {
            std::fs::read_to_string(&allowlist_path)
                .ok()
                .and_then(|d| serde_json::from_str(&d).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if !allowlist.contains(&approved.sender) {
            allowlist.push(approved.sender.clone());
            let data = serde_json::to_string_pretty(&allowlist).unwrap_or_default();
            let _ = std::fs::write(&allowlist_path, data);
        }

        info!(sender = %approved.sender, code = %code, "pairing approved for WhatsApp");
        Ok(approved.sender)
    }

    /// Deny / remove a pairing request by code.
    pub fn deny_pairing(&self, code: &str) -> claw_core::Result<()> {
        let mut requests = self.load_pairing_requests();
        if let Some(pos) = requests.iter().position(|r| r.code == code) {
            let denied = requests.remove(pos);
            self.save_pairing_requests(&requests)?;
            info!(sender = %denied.sender, code = %code, "pairing denied for WhatsApp");
        }
        Ok(())
    }

    /// Generate a short pairing code for an unknown sender.
    fn generate_pairing_code() -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let code: u32 = rng.gen_range(100_000..999_999);
        format!("{code}")
    }

    /// Load the persistent allowlist from disk.
    fn load_allowlist(&self) -> Vec<String> {
        let path = self.auth_dir.join("allow-from.json");
        if !path.exists() {
            return Vec::new();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    /// Check if a sender is allowed under the current DM policy.
    pub fn is_sender_allowed(&self, sender: &str) -> bool {
        is_sender_allowed(
            sender,
            &self.dm_policy,
            &self.allow_from,
            &self.load_allowlist(),
        )
    }

    /// Logout: clear auth state so QR re-linking is required.
    pub fn logout(&self) -> claw_core::Result<()> {
        if self.auth_dir.exists() {
            for entry in std::fs::read_dir(&self.auth_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Keep pairing.json and allow-from.json
                if name_str == "pairing.json" || name_str == "allow-from.json" {
                    continue;
                }
                let _ = std::fs::remove_file(entry.path());
            }
            info!("WhatsApp auth state cleared — re-link with QR required");
        }
        Ok(())
    }

    /// Get the bridge script directory.
    pub fn bridge_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".claw")
            .join("bridges")
            .join("whatsapp")
    }

    /// Check if the bridge is installed.
    pub fn is_bridge_installed() -> bool {
        Self::bridge_dir().join("bridge.js").exists()
            && Self::bridge_dir().join("node_modules").exists()
    }

    /// Install the WhatsApp bridge (creates bridge.js and runs npm install).
    pub fn install_bridge() -> claw_core::Result<()> {
        let dir = Self::bridge_dir();
        std::fs::create_dir_all(&dir)?;

        // Write package.json
        let package_json = json!({
            "name": "claw-whatsapp-bridge",
            "version": "1.0.0",
            "private": true,
            "type": "module",
            "dependencies": {
                "@whiskeysockets/baileys": "^6",
                "qrcode-terminal": "^0.12.0"
            }
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )?;

        // Write bridge script
        std::fs::write(dir.join("bridge.js"), WHATSAPP_BRIDGE_JS)?;

        // Run npm install
        let output = std::process::Command::new("npm")
            .args(["install", "--production"])
            .current_dir(&dir)
            .output()
            .map_err(|e| claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: format!("npm install failed: {e}. Make sure Node.js ≥ 18 is installed."),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: format!("npm install failed: {stderr}"),
            });
        }

        info!("WhatsApp bridge installed at {}", dir.display());
        Ok(())
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> &str {
        "whatsapp"
    }

    async fn start(&mut self) -> claw_core::Result<mpsc::Receiver<ChannelEvent>> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        // Check Node.js is available
        let node_check = tokio::process::Command::new("node")
            .arg("--version")
            .output()
            .await;
        if node_check.is_err() {
            return Err(claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: "Node.js not found. WhatsApp requires Node.js ≥ 18. \
                         Install from https://nodejs.org/"
                    .into(),
            });
        }

        // Auto-install bridge if not present
        if !Self::is_bridge_installed() {
            info!("WhatsApp bridge not found — installing...");
            Self::install_bridge()?;
        }

        if let Some(ref phone) = self.phone {
            info!(phone = %phone, "WhatsApp: starting with linked phone number");
        }

        let auth_dir = self.auth_dir.clone();
        let connected = self.connected.clone();
        let channel_id = self.id.clone();
        let dm_policy = self.dm_policy.clone();
        let allow_from = self.allow_from.clone();

        // Load persistent allowlist and merge with config
        let disk_allowlist = self.load_allowlist();

        tokio::spawn(async move {
            whatsapp_bridge_loop(
                auth_dir,
                channel_id,
                dm_policy,
                allow_from,
                disk_allowlist,
                event_tx,
                shutdown_rx,
                connected,
            )
            .await;
        });

        Ok(event_rx)
    }

    async fn send(&self, message: OutgoingMessage) -> claw_core::Result<()> {
        debug!(text_len = message.text.len(), target = %message.target, "WhatsApp send() called");
        debug!(text_preview = %message.text.chars().take(200).collect::<String>(), "WhatsApp send() text preview");
        if !self.is_connected() {
            return Err(claw_core::ClawError::Channel {
                channel: "whatsapp".into(),
                reason: "WhatsApp is not connected. Link your phone: claw channels login whatsapp"
                    .into(),
            });
        }

        let bridge_port = self.read_bridge_port();
        let base_url = format!("http://127.0.0.1:{bridge_port}/send");

        // ── 1. Collect image paths to send ───────────────────────
        let mut photo_paths: Vec<PathBuf> = Vec::new();
        let mut found_screenshots = false;

        // (a) /api/v1/screenshots/{name}.png URLs → ~/.claw/screenshots/
        let screenshot_files = extract_screenshot_filenames(&message.text);
        if !screenshot_files.is_empty() {
            found_screenshots = true;
            let dir = screenshots_dir();
            for filename in &screenshot_files {
                let path = dir.join(filename);
                if path.exists() {
                    debug!(path = %path.display(), "found screenshot for WhatsApp");
                    photo_paths.push(path);
                }
            }
        }

        // (b) Explicit attachments on OutgoingMessage (from channel_send_file tool)
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

        // ── 3. Send images via bridge ────────────────────────────
        let mut uploaded = 0usize;
        for photo_path in &photo_paths {
            if let Ok(bytes) = tokio::fs::read(photo_path).await {
                let b64 = base64_encode(&bytes);
                let filename = photo_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let mimetype = if filename.ends_with(".png") {
                    "image/png"
                } else if filename.ends_with(".gif") {
                    "image/gif"
                } else if filename.ends_with(".webp") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };

                let body = json!({
                    "type": "image",
                    "to": message.target,
                    "data": b64,
                    "mimetype": mimetype,
                    "filename": filename,
                });

                match self.client.post(&base_url).json(&body).send().await {
                    Ok(r) if r.status().is_success() => {
                        uploaded += 1;
                        debug!(path = %photo_path.display(), "sent image to WhatsApp");
                    }
                    Ok(r) => {
                        let t = r.text().await.unwrap_or_default();
                        warn!(error = %t, path = %photo_path.display(), "WhatsApp image send failed");
                    }
                    Err(e) => {
                        warn!(error = %e, path = %photo_path.display(), "WhatsApp image send error");
                    }
                }
            }
        }

        // ── 4. Send documents via bridge ─────────────────────────
        for doc_path in &doc_paths {
            if let Ok(bytes) = tokio::fs::read(doc_path).await {
                let b64 = base64_encode(&bytes);
                let filename = doc_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let lower = filename.to_lowercase();
                let mimetype = if lower.ends_with(".pdf") {
                    "application/pdf"
                } else if lower.ends_with(".mp3") {
                    "audio/mpeg"
                } else if lower.ends_with(".m4a") {
                    "audio/mp4"
                } else if lower.ends_with(".aac") {
                    "audio/aac"
                } else if lower.ends_with(".ogg") {
                    "audio/ogg"
                } else if lower.ends_with(".wav") {
                    "audio/wav"
                } else if lower.ends_with(".flac") {
                    "audio/flac"
                } else if lower.ends_with(".aiff") {
                    "audio/aiff"
                } else if lower.ends_with(".mp4") {
                    "video/mp4"
                } else if lower.ends_with(".mov") {
                    "video/quicktime"
                } else if lower.ends_with(".avi") {
                    "video/x-msvideo"
                } else if lower.ends_with(".mkv") {
                    "video/x-matroska"
                } else if lower.ends_with(".webm") {
                    "video/webm"
                } else if lower.ends_with(".zip") {
                    "application/zip"
                } else if lower.ends_with(".json") {
                    "application/json"
                } else if lower.ends_with(".csv") {
                    "text/csv"
                } else if lower.ends_with(".txt")
                    || lower.ends_with(".log")
                    || lower.ends_with(".md")
                {
                    "text/plain"
                } else {
                    "application/octet-stream"
                };

                // Use appropriate WhatsApp message type
                let wa_type = if mimetype.starts_with("audio/") {
                    "audio"
                } else if mimetype.starts_with("video/") {
                    "video"
                } else {
                    "document"
                };

                let body = json!({
                    "type": wa_type,
                    "to": message.target,
                    "data": b64,
                    "mimetype": mimetype,
                    "filename": filename,
                });

                match self.client.post(&base_url).json(&body).send().await {
                    Ok(r) if r.status().is_success() => {
                        uploaded += 1;
                        debug!(path = %doc_path.display(), "sent document to WhatsApp");
                    }
                    _ => {
                        warn!(path = %doc_path.display(), "WhatsApp document send failed");
                    }
                }
            }
        }

        // ── 5. Send remaining text ───────────────────────────────
        let clean_text = if found_screenshots {
            strip_file_references(&message.text, &screenshot_files, &[])
        } else {
            message.text.clone()
        };

        // If we uploaded images and text is empty, skip text send
        if uploaded > 0 && (clean_text.is_empty() || clean_text.chars().all(|c| c.is_whitespace()))
        {
            return Ok(());
        }

        // Suppress "I can't attach/send" text if we already sent the file
        if uploaded > 0 {
            let lower = clean_text.to_lowercase();
            if lower.contains("can't attach")
                || lower.contains("cannot attach")
                || lower.contains("can't upload")
                || lower.contains("cannot upload")
                || lower.contains("can't send the image")
                || lower.contains("unable to attach")
                || lower.contains("can't send the file")
                || lower.contains("cannot send the file")
                || lower.contains("can't send file")
                || lower.contains("unable to send file")
                || lower.contains("don't have a direct way to send")
                || lower.contains("cannot directly send")
            {
                return Ok(());
            }
        }

        if !clean_text.is_empty() {
            let body = json!({
                "type": "send",
                "to": message.target,
                "text": clean_text,
            });

            let resp = self.client.post(&base_url).json(&body).send().await;

            match resp {
                Ok(r) => {
                    if !r.status().is_success() {
                        let text = r.text().await.unwrap_or_default();
                        warn!(error = %text, "WhatsApp bridge send error");
                        return Err(claw_core::ClawError::Channel {
                            channel: "whatsapp".into(),
                            reason: format!("Bridge send failed: {text}"),
                        });
                    }
                }
                Err(e) => {
                    return Err(claw_core::ClawError::Channel {
                        channel: "whatsapp".into(),
                        reason: format!(
                            "Cannot reach WhatsApp bridge at port {bridge_port} — is it running? ({e})"
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    async fn send_typing(&self, target: &str) -> claw_core::Result<()> {
        if !self.is_connected() {
            return Ok(());
        }

        let bridge_port = self.read_bridge_port();
        let body = json!({ "type": "typing", "to": target });
        let _ = self
            .client
            .post(format!("http://127.0.0.1:{bridge_port}/send"))
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn stop(&mut self) -> claw_core::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        self.connected.store(false, Ordering::SeqCst);
        info!("WhatsApp channel stopped");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

impl WhatsAppChannel {
    /// Read the bridge port from the port file written by the bridge process.
    fn read_bridge_port(&self) -> u16 {
        let port_file = self.auth_dir.join("bridge.port");
        std::fs::read_to_string(&port_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(3781)
    }
}

/// Check if a sender is allowed under the given DM policy and allowlists.
fn is_sender_allowed(
    sender: &str,
    dm_policy: &DmPolicy,
    allow_from: &[String],
    disk_allowlist: &[String],
) -> bool {
    match dm_policy {
        DmPolicy::Open => true,
        DmPolicy::Disabled => false,
        DmPolicy::Allowlist | DmPolicy::Pairing => {
            allow_from.iter().any(|n| sender.contains(n))
                || disk_allowlist.iter().any(|n| sender.contains(n))
        }
    }
}

/// Bridge loop: spawns the Node.js bridge process and reads its JSON output.
#[allow(clippy::too_many_arguments)]
async fn whatsapp_bridge_loop(
    auth_dir: PathBuf,
    channel_id: String,
    dm_policy: DmPolicy,
    allow_from: Vec<String>,
    mut disk_allowlist: Vec<String>,
    event_tx: mpsc::Sender<ChannelEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    connected: Arc<AtomicBool>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let bridge_dir = WhatsAppChannel::bridge_dir();
    let bridge_script = bridge_dir.join("bridge.js");

    if !bridge_script.exists() {
        error!(
            "WhatsApp bridge script not found at {}",
            bridge_script.display()
        );
        let _ = event_tx
            .send(ChannelEvent::Disconnected(Some(
                "Bridge not installed — run: claw channels login whatsapp".into(),
            )))
            .await;
        return;
    }

    let mut backoff = 1u64;
    let mut logged_out = false;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        info!("WhatsApp: starting bridge process...");

        // Spawn Node.js bridge
        let child = tokio::process::Command::new("node")
            .arg(&bridge_script)
            .env("AUTH_DIR", auth_dir.to_string_lossy().to_string())
            .env("BRIDGE_PORT", "0") // let OS pick a port
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "WhatsApp: failed to spawn bridge");
                let _ = event_tx
                    .send(ChannelEvent::Disconnected(Some(format!(
                        "Bridge spawn failed: {e}"
                    ))))
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
        };

        backoff = 1;

        // Read stdout for JSON events from the bridge
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut bridge_connected = false;

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            let _ = child.kill().await;
                            return;
                        }
                    }
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }

                                // Bridge outputs one JSON object per line
                                let event: Value = match serde_json::from_str(trimmed) {
                                    Ok(v) => v,
                                    Err(_) => {
                                        // Non-JSON output (bridge logs)
                                        debug!("WhatsApp bridge: {}", trimmed);
                                        continue;
                                    }
                                };

                                let event_type = event["type"].as_str().unwrap_or("");

                                match event_type {
                                    "qr" => {
                                        // QR code for linking — render in terminal
                                        let qr_data = event["data"].as_str().unwrap_or("");
                                        info!("WhatsApp: QR code generated — scan with your phone");

                                        // Render QR using the qrcode crate with compact Unicode half-blocks
                                        match qrcode::QrCode::new(qr_data.as_bytes()) {
                                            Ok(code) => {
                                                let quiet = 1;
                                                let width = code.width();
                                                let colors: Vec<bool> = code.into_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
                                                let at = |x: i32, y: i32| -> bool {
                                                    if x < 0 || y < 0 || x >= width as i32 || y >= width as i32 { false }
                                                    else { colors[(y as usize) * width + (x as usize)] }
                                                };
                                                let total_w = width as i32 + quiet * 2;
                                                let total_h = width as i32 + quiet * 2;
                                                let mut qr_str = String::new();
                                                // Use Unicode half-block rendering: each character = 2 rows
                                                // ▀ (upper half), ▄ (lower half), █ (both), ' ' (neither)
                                                let mut y = -quiet;
                                                while y < total_h {
                                                    for x in -quiet..total_w - quiet {
                                                        let top = at(x, y);
                                                        let bot = at(x, y + 1);
                                                        qr_str.push(match (top, bot) {
                                                            (true, true) => '█',
                                                            (true, false) => '▀',
                                                            (false, true) => '▄',
                                                            (false, false) => ' ',
                                                        });
                                                    }
                                                    qr_str.push('\n');
                                                    y += 2;
                                                }
                                                eprintln!();
                                                eprintln!("   \x1b[1m📱 Scan this QR code with WhatsApp:\x1b[0m");
                                                eprintln!("   Open WhatsApp → Settings → Linked Devices → Link a Device\n");
                                                for line in qr_str.lines() {
                                                    eprintln!("   {line}");
                                                }
                                                eprintln!();
                                            }
                                            Err(e) => {
                                                warn!(error = %e, "Failed to render QR code");
                                                // Fallback: print raw data
                                                eprintln!("\n   QR data: {qr_data}\n");
                                            }
                                        }
                                    }
                                    "connected" => {
                                        let phone = event["phone"].as_str().unwrap_or("unknown");
                                        connected.store(true, Ordering::SeqCst);
                                        bridge_connected = true;
                                        let _ = event_tx.send(ChannelEvent::Connected).await;
                                        info!(phone = %phone, "WhatsApp connected!");

                                        // Auto-trust the linked phone number
                                        let phone_jid = format!("{phone}@s.whatsapp.net");
                                        if !allow_from.iter().any(|n| phone_jid.contains(n))
                                            && !disk_allowlist.iter().any(|n| phone_jid.contains(n))
                                        {
                                            // Persist to allow-from.json so it survives restarts
                                            let mut af = disk_allowlist.clone();
                                            af.push(phone.to_string());
                                            let af_path = auth_dir.join("allow-from.json");
                                            let _ = std::fs::write(
                                                &af_path,
                                                serde_json::to_string_pretty(&af).unwrap_or_default(),
                                            );
                                            // Also add to in-memory list
                                            disk_allowlist.push(phone.to_string());
                                            info!(phone = %phone, "WhatsApp: auto-trusted linked phone number");
                                        }

                                        // Write bridge port to file for send()
                                        if let Some(port) = event["port"].as_u64() {
                                            let port_file = auth_dir.join("bridge.port");
                                            let _ = std::fs::write(&port_file, port.to_string());
                                        }
                                    }
                                    "disconnected" => {
                                        let reason = event["reason"].as_str().unwrap_or("unknown");
                                        connected.store(false, Ordering::SeqCst);
                                        bridge_connected = false;
                                        let _ = event_tx.send(ChannelEvent::Disconnected(
                                            Some(reason.to_string())
                                        )).await;
                                        warn!(reason = %reason, "WhatsApp disconnected");

                                        if reason == "logged_out" {
                                            logged_out = true;
                                            // Kill bridge — no point retrying
                                            let _ = child.kill().await;
                                            break;
                                        }
                                    }
                                    "message" => {
                                        if !bridge_connected {
                                            continue;
                                        }

                                        let sender = event["from"].as_str().unwrap_or("");
                                        let sender_name = event["pushName"].as_str().map(|s| s.to_string());
                                        let text = event["text"].as_str().map(|s| s.to_string());
                                        let msg_id = event["id"].as_str().unwrap_or("").to_string();
                                        let is_group = event["isGroup"].as_bool().unwrap_or(false);
                                        let group_id = event["groupId"].as_str().map(|s| s.to_string());

                                        // Apply DM policy
                                        if !is_group {
                                            match &dm_policy {
                                                DmPolicy::Disabled => {
                                                    info!(sender = %sender, "WhatsApp: DM ignored (policy=disabled)");
                                                    continue;
                                                }
                                                DmPolicy::Allowlist => {
                                                    if !is_sender_allowed(sender, &dm_policy, &allow_from, &disk_allowlist) {
                                                        info!(sender = %sender, "WhatsApp: DM rejected (not in allowlist)");
                                                        continue;
                                                    }
                                                }
                                                DmPolicy::Pairing => {
                                                    if !is_sender_allowed(sender, &dm_policy, &allow_from, &disk_allowlist) {
                                                        // Generate pairing code and store request
                                                        let code = WhatsAppChannel::generate_pairing_code();
                                                        let now = chrono::Utc::now();
                                                        let expires = now + chrono::Duration::hours(24);

                                                        let req = PairingRequest {
                                                            code: code.clone(),
                                                            sender: sender.to_string(),
                                                            sender_name: sender_name.clone(),
                                                            channel: "whatsapp".into(),
                                                            created_at: now.to_rfc3339(),
                                                            expires_at: expires.to_rfc3339(),
                                                        };

                                                        // Save to disk
                                                        let pairing_path = auth_dir.join("pairing.json");
                                                        let mut existing: Vec<PairingRequest> =
                                                            if pairing_path.exists() {
                                                                std::fs::read_to_string(&pairing_path)
                                                                    .ok()
                                                                    .and_then(|d| serde_json::from_str(&d).ok())
                                                                    .unwrap_or_default()
                                                            } else {
                                                                Vec::new()
                                                            };
                                                        existing.push(req);
                                                        let data = serde_json::to_string_pretty(&existing)
                                                            .unwrap_or_default();
                                                        let _ = std::fs::write(&pairing_path, data);

                                                        info!(
                                                            sender = %sender,
                                                            code = %code,
                                                            "WhatsApp: pairing code generated for unknown sender"
                                                        );

                                                        // We skip this message — they need to be approved first
                                                        continue;
                                                    }
                                                }
                                                DmPolicy::Open => {
                                                    // Allow all
                                                }
                                            }
                                        }

                                        let incoming = IncomingMessage {
                                            id: msg_id,
                                            channel: channel_id.clone(),
                                            sender: sender.to_string(),
                                            sender_name,
                                            group: if is_group { group_id } else { None },
                                            text,
                                            attachments: parse_wa_attachments(&event),
                                            is_mention: event["mentionsMe"].as_bool().unwrap_or(false),
                                            is_reply_to_bot: false,
                                            metadata: event.clone(),
                                        };

                                        debug!(sender = %sender, "WhatsApp message received");

                                        if event_tx.send(ChannelEvent::Message(incoming)).await.is_err() {
                                            warn!("WhatsApp: event channel closed");
                                            let _ = child.kill().await;
                                            return;
                                        }
                                    }
                                    "error" => {
                                        let msg = event["message"].as_str().unwrap_or("unknown error");
                                        error!(error = %msg, "WhatsApp bridge error");
                                    }
                                    _ => {
                                        debug!(event_type = %event_type, "WhatsApp: unhandled bridge event");
                                    }
                                }
                            }
                            Ok(None) => {
                                // Bridge process exited
                                info!("WhatsApp: bridge process exited");
                                break;
                            }
                            Err(e) => {
                                warn!(error = %e, "WhatsApp: error reading bridge stdout");
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Bridge exited — wait and decide whether to restart
        let _ = child.wait().await;
        connected.store(false, Ordering::SeqCst);

        if logged_out {
            // Clear stale auth so next login starts fresh
            let creds_file = auth_dir.join("creds.json");
            if creds_file.exists() {
                let _ = std::fs::remove_file(&creds_file);
            }
            // Remove session files (app-state-sync, pre-key, sender-key, etc)
            if let Ok(entries) = std::fs::read_dir(&auth_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("app-state")
                        || name_str.starts_with("pre-key")
                        || name_str.starts_with("sender-key")
                        || name_str.starts_with("session-")
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }

            warn!("WhatsApp: session logged out by server. Auth state cleared.");
            warn!("WhatsApp: re-link your phone with: claw channels login whatsapp");

            let _ = event_tx
                .send(ChannelEvent::Disconnected(Some(
                    "Session logged out. Re-link your phone: claw channels login whatsapp".into(),
                )))
                .await;
            break;
        }

        let _ = event_tx
            .send(ChannelEvent::Disconnected(Some(
                "Bridge process exited".into(),
            )))
            .await;

        if *shutdown_rx.borrow() {
            break;
        }

        info!(retry_in = backoff, "WhatsApp: restarting bridge...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

/// Encode bytes as base64 string.
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Parse file/media attachments from a WhatsApp bridge message event.
fn parse_wa_attachments(event: &Value) -> Vec<Attachment> {
    let mut result = Vec::new();
    if let Some(media) = event.get("media") {
        let filename = media["filename"]
            .as_str()
            .unwrap_or("attachment")
            .to_string();
        let mimetype = media["mimetype"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .to_string();
        let data = media["data"].as_str().unwrap_or("").to_string();
        if !data.is_empty() {
            result.push(Attachment {
                filename,
                media_type: mimetype,
                data,
            });
        }
    }
    result
}

/// The bundled Node.js bridge script source.
/// This script uses @whiskeysockets/baileys to connect to WhatsApp Web
/// and communicates with the Rust process via JSON lines on stdout.
const WHATSAPP_BRIDGE_JS: &str = r##"
import { makeWASocket, useMultiFileAuthState, DisconnectReason, fetchLatestBaileysVersion } from '@whiskeysockets/baileys';
import { createServer } from 'http';

const AUTH_DIR = process.env.AUTH_DIR || './auth';
const BRIDGE_PORT = parseInt(process.env.BRIDGE_PORT || '0', 10);

// Emit JSON event to Rust parent process
function emit(obj) {
    process.stdout.write(JSON.stringify(obj) + '\n');
}

let sock = null;

async function start() {
    const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
    const { version } = await fetchLatestBaileysVersion();

    sock = makeWASocket({
        version,
        auth: state,
        printQRInTerminal: false,
        generateHighQualityLinkPreview: false,
    });

    sock.ev.on('creds.update', saveCreds);

    sock.ev.on('connection.update', (update) => {
        const { connection, lastDisconnect, qr } = update;

        if (qr) {
            emit({ type: 'qr', data: qr });
        }

        if (connection === 'open') {
            const phone = sock.user?.id?.split(':')[0] || 'unknown';
            emit({ type: 'connected', phone, port: httpPort });
        }

        if (connection === 'close') {
            const code = lastDisconnect?.error?.output?.statusCode;
            const reason = DisconnectReason[code] || `code ${code}`;

            if (code === DisconnectReason.loggedOut) {
                emit({ type: 'disconnected', reason: 'logged_out' });
                process.exit(0);
            } else {
                emit({ type: 'disconnected', reason });
                // Reconnect after a short delay
                setTimeout(start, 3000);
            }
        }
    });

    sock.ev.on('messages.upsert', ({ messages, type: upsertType }) => {
        if (upsertType !== 'notify') return;

        for (const msg of messages) {
            if (!msg.message) continue;

            // Allow self-messages only when sent to own number ("Message Yourself" / "Note to Self")
            const ownJid = sock.user?.id;
            const ownNumber = ownJid?.split(':')[0] || ownJid?.split('@')[0] || '';
            const isSelfChat = msg.key.remoteJid?.includes(ownNumber) || false;
            if (msg.key.fromMe && !isSelfChat) continue;

            const text =
                msg.message.conversation ||
                msg.message.extendedTextMessage?.text ||
                msg.message.imageMessage?.caption ||
                msg.message.videoMessage?.caption ||
                '';

            const isGroup = msg.key.remoteJid?.endsWith('@g.us') || false;
            const from = isGroup
                ? msg.key.participant || msg.key.remoteJid
                : msg.key.remoteJid;

            const event = {
                type: 'message',
                id: msg.key.id,
                from,
                pushName: msg.pushName || null,
                text: text || null,
                isGroup,
                groupId: isGroup ? msg.key.remoteJid : null,
                mentionsMe: msg.message.extendedTextMessage?.contextInfo?.mentionedJid?.includes(sock.user?.id) || false,
                timestamp: msg.messageTimestamp,
            };

            // Check for media
            const mediaMsg =
                msg.message.imageMessage ||
                msg.message.videoMessage ||
                msg.message.audioMessage ||
                msg.message.documentMessage;
            if (mediaMsg) {
                event.media = {
                    mimetype: mediaMsg.mimetype || 'application/octet-stream',
                    filename: mediaMsg.fileName || 'attachment',
                };
            }

            emit(event);
        }
    });
}

// Simple HTTP server for sending messages from the Rust side
let httpPort = BRIDGE_PORT;
const server = createServer(async (req, res) => {
    if (req.method === 'POST' && req.url === '/send') {
        let body = '';
        req.on('data', (chunk) => { body += chunk; });
        req.on('end', async () => {
            try {
                const data = JSON.parse(body);
                if (data.type === 'send' && sock) {
                    await sock.sendMessage(data.to, { text: data.text });
                    res.writeHead(200);
                    res.end('ok');
                } else if (data.type === 'image' && sock) {
                    const buffer = Buffer.from(data.data, 'base64');
                    await sock.sendMessage(data.to, {
                        image: buffer,
                        mimetype: data.mimetype || 'image/jpeg',
                        caption: data.caption || undefined,
                        fileName: data.filename || 'image.jpg',
                    });
                    res.writeHead(200);
                    res.end('ok');
                } else if (data.type === 'document' && sock) {
                    const buffer = Buffer.from(data.data, 'base64');
                    await sock.sendMessage(data.to, {
                        document: buffer,
                        mimetype: data.mimetype || 'application/octet-stream',
                        fileName: data.filename || 'file',
                    });
                    res.writeHead(200);
                    res.end('ok');
                } else if (data.type === 'audio' && sock) {
                    const buffer = Buffer.from(data.data, 'base64');
                    await sock.sendMessage(data.to, {
                        audio: buffer,
                        mimetype: data.mimetype || 'audio/ogg; codecs=opus',
                        ptt: data.ptt || false,
                    });
                    res.writeHead(200);
                    res.end('ok');
                } else if (data.type === 'typing' && sock) {
                    await sock.sendPresenceUpdate('composing', data.to);
                    res.writeHead(200);
                    res.end('ok');
                } else {
                    res.writeHead(400);
                    res.end('bad request');
                }
            } catch (err) {
                res.writeHead(500);
                res.end(err.message);
            }
        });
    } else {
        res.writeHead(404);
        res.end('not found');
    }
});

server.listen(httpPort, '127.0.0.1', () => {
    httpPort = server.address().port;
    start().catch((err) => {
        emit({ type: 'error', message: err.message });
        process.exit(1);
    });
});
"##;
