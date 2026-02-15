//! # PTY Terminal Session Manager
//!
//! Provides persistent pseudo-terminal (PTY) sessions that give the agent
//! a real interactive terminal — just like a human developer has a terminal
//! window open. This enables the agent to:
//!
//! - Run interactive commands that prompt for input (e.g., `npx create-next-app`)
//! - Monitor long-running processes (e.g., `npm run dev`)
//! - Send input to respond to prompts (e.g., answering "y" to confirmation questions)
//! - See exactly what a human would see in a terminal
//!
//! ## Architecture
//!
//! Each `TerminalSession` wraps a Unix PTY (pseudo-terminal) pair:
//! - **Master FD**: The "controlling" end — we read/write this to interact
//! - **Slave FD**: Connected to the child shell's stdin/stdout/stderr
//! - **Background reader**: A tokio blocking task that continuously reads
//!   output from the master fd into a ring buffer
//! - **ANSI stripping**: Terminal escape codes are stripped before returning
//!   output to the agent (it doesn't need colors/cursor control)

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Maximum output buffer size per terminal (256 KB).
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Default terminal dimensions.
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 40;

/// How long to wait for output to "settle" (no new data) before returning.
const SETTLE_MS: u64 = 500;

// ─── Output Ring Buffer ───────────────────────────────────────────────

struct OutputBuffer {
    data: Vec<u8>,
    /// Cursor marking where "new" output starts (for get_new_output).
    read_cursor: usize,
    /// Cursor marking the buffer length at the last terminal_view call.
    view_cursor: usize,
}

impl OutputBuffer {
    fn new() -> Self {
        Self {
            data: Vec::with_capacity(64 * 1024),
            read_cursor: 0,
            view_cursor: 0,
        }
    }

    /// Append bytes to the buffer, trimming the front if over max size.
    fn push(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
        if self.data.len() > MAX_OUTPUT_BYTES {
            let excess = self.data.len() - MAX_OUTPUT_BYTES;
            self.data.drain(..excess);
            self.read_cursor = self.read_cursor.saturating_sub(excess);
            self.view_cursor = self.view_cursor.saturating_sub(excess);
        }
    }

    /// Return all output since the last call to this method, then advance cursor.
    fn get_new_output(&mut self) -> String {
        if self.read_cursor >= self.data.len() {
            return String::new();
        }
        let new_data = &self.data[self.read_cursor..];
        self.read_cursor = self.data.len();
        clean_terminal_output(&strip_ansi_codes(&String::from_utf8_lossy(new_data)))
    }

    /// Return the last `n` lines of the full buffer.
    fn get_last_n_lines(&self, n: usize) -> String {
        let text = clean_terminal_output(&strip_ansi_codes(&String::from_utf8_lossy(&self.data)));
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].join("\n")
    }

    /// Current total byte count.
    fn len(&self) -> usize {
        self.data.len()
    }

    /// Bytes of new (unread) data available.
    fn new_data_len(&self) -> usize {
        self.data.len().saturating_sub(self.read_cursor)
    }

    /// Check if meaningful (non-spinner) data has arrived since the last view.
    /// Advances the view cursor to the current buffer length.
    fn has_meaningful_new_data_since_view(&mut self) -> bool {
        if self.view_cursor >= self.data.len() {
            return false;
        }
        let new_data = &self.data[self.view_cursor..];
        self.view_cursor = self.data.len();
        // Check if the new data contains anything other than Braille spinners,
        // ANSI codes, whitespace, and common spinner chars
        let text = strip_ansi_codes(&String::from_utf8_lossy(new_data));
        text.chars().any(|c| {
            !c.is_whitespace()
                && !('\u{2800}'..='\u{28FF}').contains(&c)  // Braille spinners
                && c != '\r'
                && c != '\n'
        })
    }
}

// ─── Terminal Session ─────────────────────────────────────────────────

/// A persistent PTY terminal session with a running shell.
pub struct TerminalSession {
    pub id: u32,
    pub label: String,
    master_fd: i32,
    pub child_pid: u32,
    output: Arc<Mutex<OutputBuffer>>,
    exited: Arc<AtomicBool>,
    _reader_handle: tokio::task::JoinHandle<()>,
    started_at: std::time::Instant,
}

impl TerminalSession {
    /// Check if the shell process is still alive.
    pub fn is_alive(&self) -> bool {
        if self.exited.load(Ordering::Relaxed) {
            return false;
        }
        unsafe { libc::kill(self.child_pid as i32, 0) == 0 }
    }

    /// Get uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

// ─── Terminal Manager (Global Registry) ───────────────────────────────

struct TerminalManager {
    sessions: HashMap<u32, TerminalSession>,
    next_id: u32,
}

impl TerminalManager {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 1,
        }
    }
}

/// Global terminal session registry.
static TERMINAL_REGISTRY: std::sync::LazyLock<Mutex<TerminalManager>> =
    std::sync::LazyLock::new(|| Mutex::new(TerminalManager::new()));

// ─── Public API ───────────────────────────────────────────────────────

/// Open a new persistent terminal session.
///
/// Spawns a login shell (`/bin/zsh` on macOS, `/bin/bash` fallback) connected
/// to a real PTY. Returns `(terminal_id, initial_output)`.
/// If `working_dir` is provided, the shell will `cd` into that directory after starting.
pub async fn terminal_open(label: &str, working_dir: Option<&str>) -> io::Result<(u32, String)> {
    // --- Create PTY pair ---
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;

    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }

    // --- Set terminal size ---
    let winsize = libc::winsize {
        ws_row: DEFAULT_ROWS,
        ws_col: DEFAULT_COLS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master, libc::TIOCSWINSZ, &winsize);
    }

    // --- Spawn shell with slave PTY as stdio ---
    // Use bash with no rc files to avoid zsh ZLE double-echo issues
    // and ensure clean, predictable terminal behavior for the agent.
    let shell = "/bin/bash";

    let slave_fd = slave;
    let mut cmd = std::process::Command::new(shell);
    cmd.arg("--norc").arg("--noprofile");
    // Set a simple, recognizable prompt and clean environment
    cmd.env("PS1", "$ ");
    cmd.env("TERM", "xterm-256color");
    cmd.env("CLAW_TERMINAL", "1");
    // Prevent bash from enabling bracketed paste mode (sends escape sequences)
    cmd.env("BASH_SILENCE_DEPRECATION_WARNING", "1");

    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            // Create a new session (detach from parent's controlling terminal)
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // Set the slave PTY as the controlling terminal
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            // Redirect stdin/stdout/stderr to the slave PTY
            libc::dup2(slave_fd, 0);
            libc::dup2(slave_fd, 1);
            libc::dup2(slave_fd, 2);
            // Close the original slave fd if it's not stdin/stdout/stderr
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| {
        // Clean up fds on spawn failure
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
        io::Error::other(format!("failed to spawn shell: {e}"))
    })?;
    let child_pid = child.id();

    // Close slave fd in the parent — we only use the master side
    unsafe {
        libc::close(slave);
    }

    // We don't waitpid — the shell runs independently
    // Use forget to avoid dropping Child which would close stdin pipe
    std::mem::forget(child);

    // --- Start background reader task ---
    let output = Arc::new(Mutex::new(OutputBuffer::new()));
    let output_clone = Arc::clone(&output);
    let exited = Arc::new(AtomicBool::new(false));
    let exited_clone = Arc::clone(&exited);
    let master_fd_copy = master;

    let reader_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe {
                libc::read(
                    master_fd_copy,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n <= 0 {
                // EOF or error — shell has exited
                exited_clone.store(true, Ordering::Relaxed);
                break;
            }
            let data = &buf[..n as usize];
            output_clone.blocking_lock().push(data);
        }
        debug!(pid = child_pid, "terminal reader exited");
    });

    // Wait briefly for the shell to start and print its initial prompt
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Drain the initial shell output (motd, prompt, etc.)
    let initial_output = output.lock().await.get_new_output();

    // --- Register in global registry ---
    let mut mgr = TERMINAL_REGISTRY.lock().await;
    let id = mgr.next_id;
    mgr.next_id += 1;

    mgr.sessions.insert(
        id,
        TerminalSession {
            id,
            label: label.to_string(),
            master_fd: master,
            child_pid,
            output,
            exited,
            _reader_handle: reader_handle,
            started_at: std::time::Instant::now(),
        },
    );

    info!(
        terminal_id = id,
        pid = child_pid,
        label = label,
        "opened terminal session"
    );

    // If a working directory was specified, cd into it
    if let Some(dir) = working_dir {
        let cd_cmd = format!("cd {}\n", shell_escape_str(dir));
        drop(mgr);
        terminal_write_raw(id, &cd_cmd).await?;
        // Wait for the cd to complete and discard its output
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let mgr2 = TERMINAL_REGISTRY.lock().await;
        if let Some(session) = mgr2.sessions.get(&id) {
            session.output.lock().await.get_new_output(); // discard cd output
        }
        return Ok((id, format!("(working directory: {dir})")));
    }

    Ok((id, initial_output))
}

/// Send a command to a terminal session and wait for output to settle.
///
/// Appends `\n` to the command automatically. Waits up to `timeout_ms` for
/// output to appear, then waits for output to settle (no new data for 500ms).
pub async fn terminal_run(id: u32, command: &str, timeout_ms: u64) -> io::Result<String> {
    // Send the command with a newline
    let text = format!("{command}\n");
    terminal_write_raw(id, &text).await?;

    // Wait for output to settle
    wait_for_output(id, timeout_ms).await
}

/// Send raw text to a terminal (for responding to interactive prompts).
///
/// Does NOT append `\n` — the caller must include it if needed.
/// This is useful for answering prompts like "Ok to proceed? (y/n)".
pub async fn terminal_input(id: u32, text: &str, timeout_ms: u64) -> io::Result<String> {
    terminal_write_raw(id, text).await?;
    wait_for_output(id, timeout_ms).await
}

/// Read the last N lines of terminal output (without advancing the cursor).
/// Detects repeated views with no change to prevent polling loops.
pub async fn terminal_view(id: u32, lines: usize) -> io::Result<String> {
    let mgr = TERMINAL_REGISTRY.lock().await;
    let session = mgr.sessions.get(&id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("terminal {id} not found"))
    })?;

    let status = if session.is_alive() {
        "alive"
    } else {
        "exited"
    };

    // Check if meaningful data arrived since last view (not just spinners).
    // This is reliable because it checks raw buffer bytes, not a hash of
    // a sliding window that shifts as the buffer grows.
    let has_new_meaningful = session
        .output
        .lock()
        .await
        .has_meaningful_new_data_since_view();
    let text = session.output.lock().await.get_last_n_lines(lines);

    drop(mgr);

    if !has_new_meaningful {
        return Ok(format!(
            "[terminal {id} — {status}]\n\
             [no new output since last view — process may still be working, move on to other tasks and check back later]\n"
        ));
    }

    Ok(format!("[terminal {id} — {status}]\n{text}"))
}

/// Close a terminal session (sends SIGHUP, closes master fd).
pub async fn terminal_close(id: u32) -> io::Result<String> {
    let mut mgr = TERMINAL_REGISTRY.lock().await;
    let session = mgr.sessions.remove(&id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("terminal {id} not found"))
    })?;

    let label = session.label.clone();
    let pid = session.child_pid;

    // Kill the shell process
    unsafe {
        libc::kill(pid as i32, libc::SIGHUP);
    }
    // Close master fd — this will also cause the reader task to exit
    unsafe {
        libc::close(session.master_fd);
    }

    info!(terminal_id = id, pid = pid, "closed terminal session");
    Ok(format!("Terminal {id} ('{label}', pid {pid}) closed"))
}

/// List all terminal sessions.
pub async fn terminal_list() -> Vec<TerminalInfo> {
    let mgr = TERMINAL_REGISTRY.lock().await;
    mgr.sessions
        .values()
        .map(|s| TerminalInfo {
            id: s.id,
            label: s.label.clone(),
            pid: s.child_pid,
            alive: s.is_alive(),
            uptime_secs: s.uptime_secs(),
        })
        .collect()
}

/// Summary info for a terminal session.
pub struct TerminalInfo {
    pub id: u32,
    pub label: String,
    pub pid: u32,
    pub alive: bool,
    pub uptime_secs: u64,
}

// ─── Internal Helpers ─────────────────────────────────────────────────

/// Write raw bytes to a terminal's master fd.
async fn terminal_write_raw(id: u32, text: &str) -> io::Result<()> {
    let mgr = TERMINAL_REGISTRY.lock().await;
    let session = mgr.sessions.get(&id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("terminal {id} not found"))
    })?;

    if !session.is_alive() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("terminal {id} shell has exited"),
        ));
    }

    let fd = session.master_fd;
    // Clear the "new output" buffer before writing so we only capture output
    // from this command, not leftover output from previous commands.
    session.output.lock().await.get_new_output();
    drop(mgr);

    // Small delay to let any in-flight bytes from the prompt arrive and get discarded
    // on the next clear (the background reader may have bytes in transit)
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // Drain again in case anything arrived during the delay
    {
        let mgr = TERMINAL_REGISTRY.lock().await;
        if let Some(session) = mgr.sessions.get(&id) {
            session.output.lock().await.get_new_output();
        }
    }

    let bytes = text.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || {
        let mut written = 0;
        while written < bytes.len() {
            let n = unsafe {
                libc::write(
                    fd,
                    bytes[written..].as_ptr() as *const libc::c_void,
                    bytes.len() - written,
                )
            };
            if n <= 0 {
                break;
            }
            written += n as usize;
        }
    })
    .await
    .map_err(io::Error::other)?;

    Ok(())
}

/// Wait for terminal output to settle (no new data for SETTLE_MS) or until timeout.
async fn wait_for_output(id: u32, timeout_ms: u64) -> io::Result<String> {
    let mgr = TERMINAL_REGISTRY.lock().await;
    let session = mgr.sessions.get(&id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("terminal {id} not found"))
    })?;
    let output = Arc::clone(&session.output);
    let _alive = session.is_alive();
    let exited = Arc::clone(&session.exited);
    drop(mgr);

    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_millis(timeout_ms);

    // Wait for output to appear first (up to timeout)
    loop {
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            break;
        }

        // Check if shell exited
        if exited.load(Ordering::Relaxed) {
            // Give a moment for final output to flush
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            break;
        }

        let new_len = output.lock().await.new_data_len();
        if new_len > 0 {
            // We have some new output — now wait for it to settle
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Now wait for output to settle (no new data for SETTLE_MS)
    loop {
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            break;
        }

        if exited.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            break;
        }

        let before_len = output.lock().await.len();
        let wait_time = std::cmp::min(SETTLE_MS, (deadline - elapsed).as_millis() as u64);
        tokio::time::sleep(std::time::Duration::from_millis(wait_time)).await;
        let after_len = output.lock().await.len();

        if after_len == before_len {
            // Output settled
            break;
        }
    }

    let new_output = output.lock().await.get_new_output();
    let timed_out = start.elapsed() >= deadline && !exited.load(Ordering::Relaxed);
    let status = if exited.load(Ordering::Relaxed) {
        "[process exited]\n"
    } else if timed_out {
        &format!(
            "[timed out after {}s — process still running, use terminal_view to check later]\n",
            timeout_ms / 1000
        )
    } else {
        ""
    };

    Ok(format!("{status}{new_output}"))
}

// ─── ANSI Escape Code Stripping ───────────────────────────────────────

/// Shell-escape a string for safe use in a bash command.
fn shell_escape_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Clean terminal output for the agent: strip spinner characters and collapse
/// lines that consist only of spinners into a single "[installing…]" note.
/// This prevents wasting tokens on `⠙⠹⠸⠼⠴⠦⠧⠇⠏⠋` repeated hundreds of times.
fn clean_terminal_output(s: &str) -> String {
    let mut result = Vec::new();
    let mut spinner_run = 0usize;
    for line in s.lines() {
        // A "spinner line" is one that's only Braille dots + whitespace
        let is_spinner = !line.is_empty()
            && line
                .chars()
                .all(|c| ('\u{2800}'..='\u{28FF}').contains(&c) || c.is_whitespace());
        if is_spinner {
            spinner_run += 1;
        } else {
            if spinner_run > 0 {
                result.push("[installing…]".to_string());
                spinner_run = 0;
            }
            // Also strip any inline Braille spinners (e.g. "⠙⠹⠸ added 357 packages")
            let cleaned: String = line
                .chars()
                .filter(|c| !('\u{2800}'..='\u{28FF}').contains(c))
                .collect();
            result.push(cleaned);
        }
    }
    if spinner_run > 0 {
        result.push("[installing…]".to_string());
    }
    let mut out = result.join("\n");
    // Preserve trailing newline if input had one
    if s.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Strip ANSI escape codes and other terminal control sequences from text.
/// The agent doesn't need colors, cursor movement, or other terminal formatting.
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC sequence
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ ... (params) ... final_byte
                    chars.next(); // consume '['
                    while let Some(&ch) = chars.peek() {
                        chars.next();
                        // Final byte of CSI is in range 0x40-0x7E
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] ... BEL or ESC \
                    chars.next(); // consume ']'
                    while let Some(&ch) = chars.peek() {
                        chars.next();
                        if ch == '\x07' {
                            break;
                        } // BEL
                        if ch == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some('(' | ')') => {
                    // Character set designation: ESC ( X  or  ESC ) X
                    chars.next(); // consume '(' or ')'
                    chars.next(); // consume charset designator
                }
                Some(c) if (' '..='~').contains(c) => {
                    // Other two-byte ESC sequences (e.g., ESC =, ESC >)
                    chars.next();
                }
                _ => {
                    // Unknown, skip
                    chars.next();
                }
            }
        } else if c == '\r' {
            // Skip carriage returns (terminals use \r\n, we just want \n)
            continue;
        } else if c < ' ' && c != '\n' && c != '\t' {
            // Skip other control characters (BEL, BS, etc.)
            continue;
        } else {
            result.push(c);
        }
    }

    result
}

// ─── Cleanup ──────────────────────────────────────────────────────────

/// Close all terminal sessions (call on agent shutdown).
pub async fn cleanup_all_terminals() {
    let mut mgr = TERMINAL_REGISTRY.lock().await;
    let ids: Vec<u32> = mgr.sessions.keys().copied().collect();

    for id in ids {
        if let Some(session) = mgr.sessions.remove(&id) {
            unsafe {
                libc::kill(session.child_pid as i32, libc::SIGHUP);
                libc::close(session.master_fd);
            }
            info!(
                terminal_id = id,
                pid = session.child_pid,
                "cleaned up terminal on shutdown"
            );
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_codes() {
        // Plain text passes through
        assert_eq!(strip_ansi_codes("hello world"), "hello world");

        // Color codes are stripped
        assert_eq!(strip_ansi_codes("\x1b[32mgreen\x1b[0m text"), "green text");

        // Bold, underline, etc.
        assert_eq!(strip_ansi_codes("\x1b[1mbold\x1b[22m"), "bold");

        // Cursor movement
        assert_eq!(strip_ansi_codes("\x1b[2Jhello"), "hello");

        // OSC (title setting) sequences
        assert_eq!(strip_ansi_codes("\x1b]0;My Title\x07hello"), "hello");

        // Carriage returns are stripped
        assert_eq!(strip_ansi_codes("line1\r\nline2"), "line1\nline2");

        // Mixed sequences
        assert_eq!(
            strip_ansi_codes("\x1b[33m$ \x1b[0mls\r\nfile.txt\r\n"),
            "$ ls\nfile.txt\n"
        );
    }

    #[test]
    fn test_output_buffer_push_and_read() {
        let mut buf = OutputBuffer::new();
        buf.push(b"hello ");
        buf.push(b"world\n");

        let output = buf.get_new_output();
        assert_eq!(output, "hello world\n");

        // Second read returns empty (cursor advanced)
        let output2 = buf.get_new_output();
        assert_eq!(output2, "");

        // Push more
        buf.push(b"line2\n");
        let output3 = buf.get_new_output();
        assert_eq!(output3, "line2\n");
    }

    #[test]
    fn test_output_buffer_ring() {
        let mut buf = OutputBuffer::new();
        // Push more than MAX_OUTPUT_BYTES
        let big = vec![b'A'; MAX_OUTPUT_BYTES + 1000];
        buf.push(&big);
        assert!(buf.data.len() <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn test_output_buffer_last_n_lines() {
        let mut buf = OutputBuffer::new();
        buf.push(b"line1\nline2\nline3\nline4\nline5\n");

        assert_eq!(buf.get_last_n_lines(2), "line4\nline5");
        assert_eq!(
            buf.get_last_n_lines(10),
            "line1\nline2\nline3\nline4\nline5"
        );
    }

    #[tokio::test]
    async fn test_terminal_open_close() {
        // Open a terminal
        let (id, _initial) = terminal_open("test-terminal", None)
            .await
            .expect("open failed");
        assert!(id > 0);

        // List should show it
        let list = terminal_list().await;
        assert!(list.iter().any(|t| t.id == id && t.alive));

        // Close it
        let msg = terminal_close(id).await.expect("close failed");
        assert!(msg.contains("closed"));

        // List should be empty now
        let list = terminal_list().await;
        assert!(!list.iter().any(|t| t.id == id));
    }

    #[tokio::test]
    async fn test_terminal_run_command() {
        let (id, _) = terminal_open("test-run", None).await.expect("open failed");

        // Run a simple command
        let output = terminal_run(id, "echo hello_from_pty", 5000)
            .await
            .expect("run failed");

        assert!(
            output.contains("hello_from_pty"),
            "expected 'hello_from_pty' in output: {output}"
        );

        terminal_close(id).await.ok();
    }

    #[tokio::test]
    async fn test_terminal_view() {
        let (id, _) = terminal_open("test-view", None).await.expect("open failed");

        // Run a command
        terminal_run(id, "echo view_test_line", 5000).await.ok();

        // View last lines
        let view = terminal_view(id, 10).await.expect("view failed");
        assert!(
            view.contains("view_test_line"),
            "expected 'view_test_line' in view: {view}"
        );

        terminal_close(id).await.ok();
    }
}
