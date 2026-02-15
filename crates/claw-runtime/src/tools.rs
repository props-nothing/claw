use claw_core::{Result, Tool, ToolCall, ToolResult};
use serde_json::json;
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::Mutex;
use tracing::info;

/// Info about a background process started by the agent.
#[derive(Debug, Clone)]
struct TrackedProcess {
    pid: u32,
    label: String,
    command: String,
    log_file: String,
    started_at: std::time::Instant,
}

/// Global registry of background processes started by the agent.
static PROCESS_REGISTRY: LazyLock<Mutex<HashMap<u32, TrackedProcess>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Track how many times file_edit has been called on each file path
/// within a session. After 2+ edits on the same file, we warn the model
/// to consider using file_write instead (prevent incremental edit spirals).
static FILE_EDIT_COUNTS: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Built-in tools that ship with the Claw runtime.
#[derive(Clone, Copy)]
pub struct BuiltinTools;

impl BuiltinTools {
    pub fn new() -> Self {
        Self
    }

    pub fn has_tool(&self, name: &str) -> bool {
        matches!(
            name,
            "shell_exec"
                | "file_read"
                | "file_write"
                | "file_edit"
                | "file_list"
                | "file_find"
                | "file_grep"
                | "http_fetch"
                | "web_search"
                | "memory_search"
                | "memory_store"
                | "memory_delete"
                | "memory_list"
                | "goal_create"
                | "goal_list"
                | "goal_complete_step"
                | "goal_update_status"
                | "mesh_peers"
                | "mesh_delegate"
                | "mesh_status"
                | "process_start"
                | "process_list"
                | "process_kill"
                | "process_output"
                | "apply_patch"
                | "terminal_open"
                | "terminal_run"
                | "terminal_view"
                | "terminal_input"
                | "terminal_close"
                | "channel_send_file"
                | "sub_agent_spawn"
                | "sub_agent_wait"
                | "sub_agent_status"
                | "cron_schedule"
        )
    }

    pub fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "shell_exec".into(),
                description: "Run a quick, non-interactive shell command (ls, cat, mkdir, grep, git status, etc.) and return stdout/stderr. Stdin is /dev/null — cannot handle prompts. For interactive or long-running commands (npm install, npx create-*, dev servers, anything that prompts), use terminal_open + terminal_run instead. Default timeout: 120s.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute. Must be non-interactive (no TTY). Use --yes/-y flags for commands that prompt."
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Working directory (optional)"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 120). Use 300 for large installs."
                        }
                    },
                    "required": ["command"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 6,
                provider: None,
            },
            Tool {
                name: "file_read".into(),
                description: "Read the contents of a file".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read"
                        }
                    },
                    "required": ["path"]
                }),
                capabilities: vec!["fs.read".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "file_write".into(),
                description: "Write content to a file (creates or overwrites). Use this for creating new files and for replacing most or all of a file's content. Write complete, production-quality content — not stubs or placeholders. Prefer this over file_edit when changing more than a few lines.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to write"
                        },
                        "content": {
                            "type": "string",
                            "description": "Content to write"
                        }
                    },
                    "required": ["path", "content"]
                }),
                capabilities: vec!["fs.write".into()],
                is_mutating: true,
                risk_level: 4,
                provider: None,
            },
            Tool {
                name: "file_list".into(),
                description: "List files and directories at a path".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory path to list"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Whether to list recursively"
                        }
                    },
                    "required": ["path"]
                }),
                capabilities: vec!["fs.read".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "file_edit".into(),
                description: "Surgical search-and-replace edit. Finds the EXACT old_string in the file and replaces the first occurrence with new_string. Best for changing a few lines in a large file you want to keep intact. If the replacement covers more than ~30% of the file, you should use file_write instead — this tool will warn you. If this tool fails twice on the same file, switch to file_write and rewrite the file cleanly.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to edit"
                        },
                        "old_string": {
                            "type": "string",
                            "description": "The exact text to find in the file (must match exactly, including whitespace)"
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The text to replace old_string with"
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                }),
                capabilities: vec!["fs.write".into()],
                is_mutating: true,
                risk_level: 3,
                provider: None,
            },
            Tool {
                name: "file_find".into(),
                description: "Find files matching a glob pattern recursively. Returns file paths. Use this to discover project structure and locate files before reading them.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "directory": {
                            "type": "string",
                            "description": "Root directory to search from"
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern to match (e.g. '*.rs', '**/*.ts', 'src/**/*.py')"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 100)"
                        }
                    },
                    "required": ["directory", "pattern"]
                }),
                capabilities: vec!["fs.read".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "file_grep".into(),
                description: "Search file contents for a regex pattern across a directory tree. Returns matching lines with file paths and line numbers. Use this to find code, function definitions, imports, or any text pattern.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "directory": {
                            "type": "string",
                            "description": "Root directory to search in"
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for (e.g. 'fn main', 'import.*React', 'TODO|FIXME')"
                        },
                        "file_pattern": {
                            "type": "string",
                            "description": "Optional glob to filter files (e.g. '*.rs', '*.ts'). Default: all text files."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum matching lines to return (default: 50)"
                        }
                    },
                    "required": ["directory", "pattern"]
                }),
                capabilities: vec!["fs.read".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "process_start".into(),
                description: "Start a background process (e.g. dev server, watcher, long install). Returns a PID. Output is captured to a log file — use `process_output` to read it. Stdin is piped from /dev/null, so commands must be non-interactive. IMPORTANT: Only start a dev server ONCE. If one is already running, do NOT kill and restart it — just continue editing files. The running server will pick up changes automatically (hot reload). Always set working_dir to the project root.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to run in the background"
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Working directory (optional)"
                        },
                        "label": {
                            "type": "string",
                            "description": "Human-readable label for this process (e.g. 'dev-server', 'watcher')"
                        }
                    },
                    "required": ["command"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 5,
                provider: None,
            },
            Tool {
                name: "process_list".into(),
                description: "List all background processes started by the agent, with their PIDs, status, labels, and uptime. Shows whether each process is still running.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "process_kill".into(),
                description: "Kill a background process by PID. WARNING: Do NOT kill dev servers that are running correctly. Modern dev servers (Next.js, Vite, etc.) have hot-reload — just edit files and the server picks up changes automatically. Only kill a process if it has truly errored or you need the port for a different purpose.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pid": {
                            "type": "integer",
                            "description": "Process ID to kill"
                        }
                    },
                    "required": ["pid"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 4,
                provider: None,
            },
            Tool {
                name: "process_output".into(),
                description: "Read the stdout/stderr output of a background process started with process_start. Shows the last N lines (default: 50). Use this to check if a process succeeded, failed, or is still running.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pid": {
                            "type": "integer",
                            "description": "Process ID to read output from"
                        },
                        "lines": {
                            "type": "integer",
                            "description": "Number of lines to read from the end (default: 50)"
                        }
                    },
                    "required": ["pid"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "llm_generate".into(),
                description: "Generate text using the LLM. Useful for summarization, synthesis, analysis, and content generation.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The prompt/instruction to send to the LLM"
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Maximum tokens to generate (default: 2048)"
                        }
                    },
                    "required": ["prompt"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "memory_search".into(),
                description: "Search the agent's long-term memory for relevant facts, past conversations, and learned lessons. Searches across category names, keys, and values using word-level matching. Use short keywords rather than full sentences for best results. Use memory_list if you want to browse everything stored.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search keywords (e.g. 'plesk login', 'user preferences', 'server config')"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["episodic", "semantic", "all"],
                            "description": "Memory type to search. Default: all"
                        }
                    },
                    "required": ["query"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "memory_store".into(),
                description: "Store a fact or piece of knowledge in long-term memory. Use category 'learned_lessons' for things you discovered through trial-and-error or user corrections (e.g. how a specific UI works, correct form values, multi-step procedures). These lessons are automatically loaded in future sessions.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "category": {
                            "type": "string",
                            "description": "Category for the fact (e.g., 'user_preferences', 'project_info')"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key/name for the fact"
                        },
                        "value": {
                            "type": "string",
                            "description": "The fact/knowledge to store"
                        }
                    },
                    "required": ["category", "key", "value"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "memory_delete".into(),
                description: "Delete a fact from long-term memory by category and key, or delete an entire category. Use memory_list or memory_search first to find the exact category/key to delete.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "category": {
                            "type": "string",
                            "description": "Category of the fact to delete"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key of the specific fact to delete. Omit to delete ALL facts in the category."
                        }
                    },
                    "required": ["category"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 2,
                provider: None,
            },
            Tool {
                name: "memory_list".into(),
                description: "List all stored facts in long-term memory, optionally filtered by category. Use this to see what's in memory before searching, deleting, or updating.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "category": {
                            "type": "string",
                            "description": "Optional: only list facts in this category. Omit to list all categories and their facts."
                        }
                    }
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "goal_create".into(),
                description: "Create a new goal for the agent to pursue. IMPORTANT: Always run goal_list first to check for existing goals before creating a new one — do NOT create duplicate goals. If a matching goal already exists, use its ID instead of creating a new one.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "description": {
                            "type": "string",
                            "description": "What the goal is"
                        },
                        "priority": {
                            "type": "integer",
                            "description": "Priority 1-10 (10 = highest)"
                        },
                        "steps": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Planned steps to achieve the goal"
                        }
                    },
                    "required": ["description"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "goal_list".into(),
                description: "List all active goals and their progress".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "goal_complete_step".into(),
                description: "Mark a goal step as completed with a result. Automatically updates goal progress. Use goal_list first to get the goal_id and step_id UUIDs.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "The goal UUID"
                        },
                        "step_id": {
                            "type": "string",
                            "description": "The step UUID to mark complete"
                        },
                        "result": {
                            "type": "string",
                            "description": "What was accomplished in this step"
                        }
                    },
                    "required": ["goal_id", "step_id", "result"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "goal_update_status".into(),
                description: "Update a goal's status (e.g., to 'completed', 'failed', 'paused', 'cancelled'). Use this when a goal is finished, no longer relevant, or needs to be paused.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "The goal UUID"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["completed", "failed", "paused", "cancelled", "active"],
                            "description": "New status for the goal"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Why the status is being changed"
                        }
                    },
                    "required": ["goal_id", "status"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "web_search".into(),
                description: "Search the web using Brave Search API. Returns titles, URLs, and descriptions of matching results. Requires services.brave_api_key to be configured.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of results to return (default: 5, max: 20)"
                        }
                    },
                    "required": ["query"]
                }),
                capabilities: vec!["network".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "http_fetch".into(),
                description: "Fetch a URL and return its content as text. Use this to read web pages, API responses, documentation, or any HTTP resource. Essential for researching reference URLs the user provides (e.g., 'rebuild this website'). Strips HTML to readable text by default. Use max_bytes to limit response size.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to fetch"
                        },
                        "max_bytes": {
                            "type": "integer",
                            "description": "Maximum response size in bytes (default: 50000)"
                        }
                    },
                    "required": ["url"]
                }),
                capabilities: vec!["network".into()],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            Tool {
                name: "mesh_peers".into(),
                description: "List all connected mesh peers with their capabilities, hostnames, and OS. Use this to discover which peers are available and what they can do before delegating tasks.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "capability": {
                            "type": "string",
                            "description": "Filter peers by capability (e.g., 'gpu', 'browser', 'shell'). If omitted, returns all peers."
                        }
                    }
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "mesh_delegate".into(),
                description: "Delegate a task to a mesh peer. The peer will execute the task and return the result. Use mesh_peers first to find a peer with the right capabilities. You can delegate by specifying a peer_id directly, or by required_capability to auto-select the best peer.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Description of the task to delegate (natural language, will be processed by the peer's LLM)"
                        },
                        "peer_id": {
                            "type": "string",
                            "description": "Specific peer ID to delegate to (optional if capability is provided)"
                        },
                        "capability": {
                            "type": "string",
                            "description": "Required capability — will auto-select the best available peer with this capability"
                        },
                        "priority": {
                            "type": "integer",
                            "description": "Priority 0-10 (default: 5, higher = more urgent)"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "How long to wait for the result (default: 120 seconds)"
                        }
                    },
                    "required": ["task"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 3,
                provider: None,
            },
            Tool {
                name: "mesh_status".into(),
                description: "Get the status of the mesh network: whether it's running, our peer ID, how many peers are connected, and our capabilities.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "apply_patch".into(),
                description: "Apply multiple surgical edits across one or more files in a single operation. Each edit is a search-and-replace like file_edit. More efficient than multiple file_edit calls. Best for coordinated small changes (e.g., renaming a variable across files). For rewriting entire files, use file_write instead.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "edits": {
                            "type": "array",
                            "description": "List of edit operations to apply",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {
                                        "type": "string",
                                        "description": "File path to edit"
                                    },
                                    "old_string": {
                                        "type": "string",
                                        "description": "Exact text to find"
                                    },
                                    "new_string": {
                                        "type": "string",
                                        "description": "Text to replace with"
                                    }
                                },
                                "required": ["path", "old_string", "new_string"]
                            }
                        }
                    },
                    "required": ["edits"]
                }),
                capabilities: vec!["fs.write".into()],
                is_mutating: true,
                risk_level: 4,
                provider: None,
            },
            // ── Terminal (PTY) Tools ──────────────────────────────────────
            Tool {
                name: "terminal_open".into(),
                description: "Open a persistent PTY terminal session. Returns a terminal_id. Use terminals for anything interactive or long-running: npx create-*, npm install, dev servers, build watchers, commands that prompt for input. Set working_dir to start in the right directory. Use one terminal per purpose (e.g., one for install, one for dev server).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Optional label for this terminal (e.g. 'dev-server', 'install')"
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Directory to start the terminal in (optional). The shell will cd here before returning."
                        }
                    }
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 3,
                provider: None,
            },
            Tool {
                name: "terminal_run".into(),
                description: "Run a command in a terminal and return output. Waits for output to settle (no new data for 500ms). Default timeout is 120s; use timeout_secs: 180 for slow installs. If output says '[timed out]', the process is still running — check with terminal_view later, don't re-run. Dev servers never finish — read the first output to confirm startup, then move on.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "terminal_id": {
                            "type": "integer",
                            "description": "Terminal session ID (from terminal_open)"
                        },
                        "command": {
                            "type": "string",
                            "description": "The command to run"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Max seconds to wait for output to settle (default: 120). Commands like npm install may need 180-300."
                        }
                    },
                    "required": ["terminal_id", "command"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 5,
                provider: None,
            },
            Tool {
                name: "terminal_view".into(),
                description: "View recent terminal output. Use this to check on running processes or see output you missed. If it says 'no new output since last view', stop checking and move on to other work — come back later if needed.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "terminal_id": {
                            "type": "integer",
                            "description": "Terminal session ID"
                        },
                        "lines": {
                            "type": "integer",
                            "description": "Number of lines to show (default: 50)"
                        }
                    },
                    "required": ["terminal_id"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "terminal_input".into(),
                description: "Send raw text/keystrokes to a terminal. Use this to respond to interactive prompts (e.g., send 'y\\n' to confirm, 'n\\n' to decline, or typed input to fill forms). Does NOT auto-append a newline — include \\n if you need to press Enter.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "terminal_id": {
                            "type": "integer",
                            "description": "Terminal session ID"
                        },
                        "text": {
                            "type": "string",
                            "description": "Text to send (include \\n to press Enter). For Ctrl+C send \\u0003."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Max seconds to wait for response output (default: 10)"
                        }
                    },
                    "required": ["terminal_id", "text"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 3,
                provider: None,
            },
            Tool {
                name: "terminal_close".into(),
                description: "Close a terminal session and kill the shell process. Use when done with a terminal.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "terminal_id": {
                            "type": "integer",
                            "description": "Terminal session ID to close"
                        }
                    },
                    "required": ["terminal_id"]
                }),
                capabilities: vec!["shell".into()],
                is_mutating: true,
                risk_level: 2,
                provider: None,
            },
            Tool {
                name: "channel_send_file".into(),
                description: "Send a file (document, audio, image, video) to the user through the current chat channel (Telegram, WhatsApp, etc.). Use this whenever the user asks you to send, share, or deliver a file. The file will be uploaded and delivered as a native attachment in the chat.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Absolute path to the file to send (e.g. /Users/name/Desktop/report.pdf, ~/Music/song.mp3)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Optional caption/message to include with the file"
                        }
                    },
                    "required": ["file_path"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 1,
                provider: None,
            },
            // ── Sub-Agent Tools ───────────────────────────────────────
            Tool {
                name: "sub_agent_spawn".into(),
                description: "Spawn a sub-agent to work on a specific task concurrently. The sub-agent runs in its own session with a role-specific system prompt. Returns immediately with a task_id — use sub_agent_wait to collect results. Before spawning, generate a context_summary describing what the sub-agent needs to know from the current conversation. For complex projects, spawn multiple sub-agents with depends_on to create a task pipeline (e.g., coder depends on planner, reviewer depends on coder).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "role": {
                            "type": "string",
                            "description": "Role for the sub-agent (e.g., 'planner', 'coder', 'reviewer', 'researcher', 'tester', 'devops'). Determines the system prompt personality."
                        },
                        "task": {
                            "type": "string",
                            "description": "Detailed task description for the sub-agent to execute"
                        },
                        "context_summary": {
                            "type": "string",
                            "description": "Summary of relevant context from the parent conversation that the sub-agent needs to know"
                        },
                        "depends_on": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of sub-agent task_ids that must complete before this sub-agent starts. The sub-agent will wait for all dependencies and receive their results."
                        },
                        "model": {
                            "type": "string",
                            "description": "Optional model override for this sub-agent (e.g., use a faster model for simple tasks)"
                        },
                        "goal_id": {
                            "type": "string",
                            "description": "Optional goal UUID to link this sub-agent to. When provided with step_id, the goal step will be automatically marked as completed when the sub-agent finishes (or failed if it errors). Get goal/step IDs from goal_list."
                        },
                        "step_id": {
                            "type": "string",
                            "description": "Optional step UUID within the goal to link this sub-agent to. Must be used together with goal_id."
                        }
                    },
                    "required": ["role", "task"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 2,
                provider: None,
            },
            Tool {
                name: "sub_agent_wait".into(),
                description: "Wait for one or more sub-agent tasks to complete and return their results. Blocks until all specified tasks are done. Use this after spawning sub-agents to collect their output before proceeding.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of sub-agent task_ids to wait for"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Maximum seconds to wait (default: 0 = unlimited)"
                        }
                    },
                    "required": ["task_ids"]
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            Tool {
                name: "sub_agent_status".into(),
                description: "Check the status of sub-agent tasks without blocking. Returns the current status (pending, running, waiting_for_deps, completed, failed) and result if available.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of sub-agent task_ids to check (if empty, returns all)"
                        }
                    }
                }),
                capabilities: vec![],
                is_mutating: false,
                risk_level: 0,
                provider: None,
            },
            // ── Scheduler Tools ───────────────────────────────────────
            Tool {
                name: "cron_schedule".into(),
                description: "Schedule a task to run later. Supports recurring cron expressions (e.g., '*/5 * * * *' for every 5 minutes) or one-shot delayed execution. Use this for: recurring health checks, auto-resuming interrupted work, reminders, scheduled builds, periodic monitoring. The scheduled task will create a new agent session and execute the description as a prompt.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "description": {
                            "type": "string",
                            "description": "The task description / prompt that will be executed when the schedule fires"
                        },
                        "cron_expr": {
                            "type": "string",
                            "description": "Cron expression for recurring tasks (e.g., '0 */6 * * *' for every 6 hours). Mutually exclusive with delay_seconds."
                        },
                        "delay_seconds": {
                            "type": "integer",
                            "description": "Fire once after this many seconds (e.g., 300 for 5 minutes). Mutually exclusive with cron_expr."
                        },
                        "label": {
                            "type": "string",
                            "description": "Optional human-readable label for this scheduled task"
                        }
                    },
                    "required": ["description"]
                }),
                capabilities: vec![],
                is_mutating: true,
                risk_level: 2,
                provider: None,
            },

        ]
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResult> {
        match call.tool_name.as_str() {
            "shell_exec" => self.exec_shell(call).await,
            "file_read" => self.exec_file_read(call).await,
            "file_write" => self.exec_file_write(call).await,
            "file_edit" => self.exec_file_edit(call).await,
            "file_list" => self.exec_file_list(call).await,
            "file_find" => self.exec_file_find(call).await,
            "file_grep" => self.exec_file_grep(call).await,
            "http_fetch" => self.exec_http_fetch(call).await,
            "process_start" => self.exec_process_start(call).await,
            "process_list" => self.exec_process_list(call).await,
            "process_kill" => self.exec_process_kill(call).await,
            "process_output" => self.exec_process_output(call).await,
            "apply_patch" => self.exec_apply_patch(call).await,
            "terminal_open" => self.exec_terminal_open(call).await,
            "terminal_run" => self.exec_terminal_run(call).await,
            "terminal_view" => self.exec_terminal_view(call).await,
            "terminal_input" => self.exec_terminal_input(call).await,
            "terminal_close" => self.exec_terminal_close(call).await,
            _ => Err(claw_core::ClawError::ToolNotFound(call.tool_name.clone())),
        }
    }

    async fn exec_shell(&self, call: &ToolCall) -> Result<ToolResult> {
        let command = call.arguments["command"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "shell_exec".into(),
                reason: "missing 'command' argument".into(),
            }
        })?;

        let timeout_secs = call.arguments["timeout_secs"].as_u64().unwrap_or(120);
        let working_dir = call.arguments["working_dir"].as_str();

        info!(
            command = command,
            timeout_secs = timeout_secs,
            "executing shell command"
        );

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        // Pipe stdin to /dev/null so interactive commands fail fast instead of hanging
        cmd.stdin(std::process::Stdio::null());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        let output =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
                .await
                .map_err(|_| claw_core::ClawError::ToolExecution {
                    tool: "shell_exec".into(),
                    reason: format!("command timed out after {}s", timeout_secs),
                })?
                .map_err(|e| claw_core::ClawError::ToolExecution {
                    tool: "shell_exec".into(),
                    reason: e.to_string(),
                })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        let content = format!(
            "Exit code: {}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            exit_code,
            stdout.chars().take(10_000).collect::<String>(),
            stderr.chars().take(5_000).collect::<String>(),
        );

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content,
            is_error: !output.status.success(),
            data: Some(json!({ "exit_code": exit_code })),
        })
    }

    async fn exec_file_read(&self, call: &ToolCall) -> Result<ToolResult> {
        let path =
            call.arguments["path"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "file_read".into(),
                    reason: "missing 'path' argument".into(),
                })?;

        match tokio::fs::read_to_string(path).await {
            Ok(content) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: content.chars().take(50_000).collect(),
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error reading {}: {}", path, e),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_file_write(&self, call: &ToolCall) -> Result<ToolResult> {
        let path =
            call.arguments["path"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "file_write".into(),
                    reason: "missing 'path' argument".into(),
                })?;
        let content = call.arguments["content"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_write".into(),
                reason: "missing 'content' argument".into(),
            }
        })?;

        // Create parent directories
        if let Some(parent) = std::path::Path::new(path).parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        match tokio::fs::write(path, content).await {
            Ok(_) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Successfully wrote {} bytes to {}", content.len(), path),
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error writing {}: {}", path, e),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_file_list(&self, call: &ToolCall) -> Result<ToolResult> {
        let path =
            call.arguments["path"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "file_list".into(),
                    reason: "missing 'path' argument".into(),
                })?;

        let mut entries = Vec::new();
        let mut dir =
            tokio::fs::read_dir(path)
                .await
                .map_err(|e| claw_core::ClawError::ToolExecution {
                    tool: "file_list".into(),
                    reason: e.to_string(),
                })?;

        while let Some(entry) =
            dir.next_entry()
                .await
                .map_err(|e| claw_core::ClawError::ToolExecution {
                    tool: "file_list".into(),
                    reason: e.to_string(),
                })?
        {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().await.ok();
            let prefix = if file_type.map(|ft| ft.is_dir()).unwrap_or(false) {
                "📁 "
            } else {
                "📄 "
            };
            entries.push(format!("{}{}", prefix, name));
        }

        entries.sort();
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: entries.join("\n"),
            is_error: false,
            data: None,
        })
    }

    async fn exec_http_fetch(&self, call: &ToolCall) -> Result<ToolResult> {
        let url =
            call.arguments["url"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "http_fetch".into(),
                    reason: "missing 'url' argument".into(),
                })?;

        let max_bytes = call.arguments["max_bytes"].as_u64().unwrap_or(50_000) as usize;

        info!(url = url, "fetching URL");

        // Use curl via shell for simplicity — no extra HTTP client dependency needed
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(format!(
            "curl -sL --max-time 30 '{}'",
            url.replace('\'', "'\\''")
        ));

        let output = tokio::time::timeout(std::time::Duration::from_secs(35), cmd.output())
            .await
            .map_err(|_| claw_core::ClawError::ToolExecution {
                tool: "http_fetch".into(),
                reason: "request timed out".into(),
            })?
            .map_err(|e| claw_core::ClawError::ToolExecution {
                tool: "http_fetch".into(),
                reason: e.to_string(),
            })?;

        let body = String::from_utf8_lossy(&output.stdout);
        let content: String = body.chars().take(max_bytes).collect();

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content,
            is_error: !output.status.success(),
            data: None,
        })
    }

    // ── file_edit: surgical search-and-replace ─────────────────

    async fn exec_file_edit(&self, call: &ToolCall) -> Result<ToolResult> {
        let path =
            call.arguments["path"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "file_edit".into(),
                    reason: "missing 'path' argument".into(),
                })?;
        let old_string = call.arguments["old_string"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_edit".into(),
                reason: "missing 'old_string' argument".into(),
            }
        })?;
        let new_string = call.arguments["new_string"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_edit".into(),
                reason: "missing 'new_string' argument".into(),
            }
        })?;

        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Error reading {}: {}", path, e),
                    is_error: true,
                    data: None,
                });
            }
        };

        // Smart guard: if old_string covers >50% of the file, nudge toward file_write
        let coverage = if content.len() > 0 {
            old_string.len() as f64 / content.len() as f64
        } else {
            1.0
        };
        if coverage > 0.5 {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "The old_string covers {:.0}% of {} — that's a rewrite, not a surgical edit. \
                     Use file_write to replace the whole file instead.",
                    coverage * 100.0,
                    path
                ),
                is_error: true,
                data: None,
            });
        }

        let occurrences = content.matches(old_string).count();
        if occurrences == 0 {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Error: old_string not found in {}. Make sure you're matching the exact text including whitespace and indentation. \
                     TIP: If you're trying to restructure or rewrite most of the file, use `file_write` instead of `file_edit`.",
                    path
                ),
                is_error: true,
                data: None,
            });
        }

        // Track per-file edit count and warn on repeated edits
        let edit_count = {
            let mut counts = FILE_EDIT_COUNTS.lock().await;
            let count = counts.entry(path.to_string()).or_insert(0);
            *count += 1;
            *count
        };

        // Replace first occurrence only (like most code editors)
        let new_content = content.replacen(old_string, new_string, 1);
        match tokio::fs::write(path, &new_content).await {
            Ok(_) => {
                let mut msg = format!(
                    "Successfully edited {} ({} occurrence{} found, replaced first)",
                    path,
                    occurrences,
                    if occurrences == 1 { "" } else { "s" }
                );
                if edit_count >= 2 {
                    msg.push_str(&format!(
                        "\n\n⚠️ You've edited this file {} times. Consider using file_write to write the complete file \
                         content in one shot instead of many small edits.",
                        edit_count
                    ));
                }
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: msg,
                    is_error: false,
                    data: Some(json!({ "occurrences": occurrences, "edit_count": edit_count })),
                })
            }
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error writing {}: {}", path, e),
                is_error: true,
                data: None,
            }),
        }
    }

    // ── file_find: recursive glob search ───────────────────────

    async fn exec_file_find(&self, call: &ToolCall) -> Result<ToolResult> {
        let directory = call.arguments["directory"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_find".into(),
                reason: "missing 'directory' argument".into(),
            }
        })?;
        let pattern = call.arguments["pattern"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_find".into(),
                reason: "missing 'pattern' argument".into(),
            }
        })?;
        let max_results = call.arguments["max_results"].as_u64().unwrap_or(100) as usize;

        // Exclusion list for common junk directories
        let excludes = r"\( -name node_modules -o -name .git -o -name .next -o -name dist -o -name build -o -name target -o -name __pycache__ -o -name .cache \) -prune -o";

        // Translate glob patterns for `find`:
        // - `**` is not understood by `find`, convert to just `*`
        // - `**/foo` becomes find -name 'foo' (any depth)
        let clean_pattern = pattern.replace("**", "*");
        // Remove leading */ — find already recurses
        let clean_pattern = clean_pattern.trim_start_matches("*/");

        let glob = if clean_pattern.contains('/') {
            // Path-style glob — use find with -path
            format!(
                "find {} {} -path '{}' -type f -print 2>/dev/null | head -n {}",
                shell_escape(directory),
                excludes,
                shell_escape(clean_pattern),
                max_results
            )
        } else {
            format!(
                "find {} {} -name '{}' -type f -print 2>/dev/null | head -n {}",
                shell_escape(directory),
                excludes,
                shell_escape(clean_pattern),
                max_results
            )
        };

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&glob)
            .output()
            .await
            .map_err(|e| claw_core::ClawError::ToolExecution {
                tool: "file_find".into(),
                reason: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

        if files.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("No files found matching '{}' in {}", pattern, directory),
                is_error: false,
                data: None,
            });
        }

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Found {} files:\n{}", files.len(), files.join("\n")),
            is_error: false,
            data: Some(json!({ "count": files.len() })),
        })
    }

    // ── file_grep: search file contents ────────────────────────

    async fn exec_file_grep(&self, call: &ToolCall) -> Result<ToolResult> {
        let directory = call.arguments["directory"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_grep".into(),
                reason: "missing 'directory' argument".into(),
            }
        })?;
        let pattern = call.arguments["pattern"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "file_grep".into(),
                reason: "missing 'pattern' argument".into(),
            }
        })?;
        let file_pattern = call.arguments["file_pattern"].as_str();
        let max_results = call.arguments["max_results"].as_u64().unwrap_or(50) as usize;

        // Exclude common junk directories
        let excludes = "--exclude-dir=node_modules --exclude-dir=.git --exclude-dir=.next --exclude-dir=dist --exclude-dir=build --exclude-dir=target --exclude-dir=__pycache__ --exclude-dir=.cache --exclude-dir=vendor --exclude-dir=.venv";

        // Build grep command with optional file filter
        let cmd = if let Some(fp) = file_pattern {
            format!(
                "grep -rn {} --include='{}' -E {} {} 2>/dev/null | head -n {}",
                excludes,
                shell_escape(fp),
                shell_escape(pattern),
                shell_escape(directory),
                max_results
            )
        } else {
            format!(
                "grep -rn {} -I -E {} {} 2>/dev/null | head -n {}",
                excludes,
                shell_escape(pattern),
                shell_escape(directory),
                max_results
            )
        };

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .await
            .map_err(|e| claw_core::ClawError::ToolExecution {
                tool: "file_grep".into(),
                reason: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

        if lines.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("No matches for '{}' in {}", pattern, directory),
                is_error: false,
                data: None,
            });
        }

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("{} matches:\n{}", lines.len(), lines.join("\n")),
            is_error: false,
            data: Some(json!({ "count": lines.len() })),
        })
    }

    // ── process_start: launch background process with output capture ──

    async fn exec_process_start(&self, call: &ToolCall) -> Result<ToolResult> {
        let command = call.arguments["command"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "process_start".into(),
                reason: "missing 'command' argument".into(),
            }
        })?;
        let working_dir = call.arguments["working_dir"].as_str();
        let label = call.arguments["label"].as_str().unwrap_or("background");

        info!(
            command = command,
            label = label,
            "starting background process"
        );

        // Create a temp log file for capturing output
        let log_file = format!("/tmp/claw-proc-{}.log", uuid::Uuid::new_v4().as_simple());

        // Redirect stdout+stderr to log file, pipe stdin from /dev/null
        let wrapped_command = format!("{} > {} 2>&1 < /dev/null", command, shell_escape(&log_file));

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&wrapped_command);
        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }
        // Fully detach — stdin/stdout/stderr all null since we redirect in shell
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);
                // Drop the child handle — the OS process continues independently
                std::mem::forget(child);

                // Track the process in the registry
                PROCESS_REGISTRY.lock().await.insert(
                    pid,
                    TrackedProcess {
                        pid,
                        label: label.to_string(),
                        command: command.to_string(),
                        log_file: log_file.clone(),
                        started_at: std::time::Instant::now(),
                    },
                );

                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Started background process '{}' (PID: {})\n\
                         Command: {}\n\
                         Log file: {}\n\
                         Use `process_output` with PID {} to check output.",
                        label, pid, command, log_file, pid
                    ),
                    is_error: false,
                    data: Some(json!({ "pid": pid, "label": label, "log_file": log_file })),
                })
            }
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to start process: {}", e),
                is_error: true,
                data: None,
            }),
        }
    }

    // ── process_list: list tracked background processes ────────

    async fn exec_process_list(&self, call: &ToolCall) -> Result<ToolResult> {
        let registry = PROCESS_REGISTRY.lock().await;

        if registry.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: "No background processes tracked. Use `process_start` to launch one."
                    .into(),
                is_error: false,
                data: None,
            });
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "{:<8} {:<6} {:<12} {:<10} {}",
            "PID", "ALIVE", "LABEL", "UPTIME", "COMMAND"
        ));
        lines.push("─".repeat(70));

        for proc in registry.values() {
            // Check if process is still running
            let alive = is_process_alive(proc.pid);
            let status = if alive { "✅ yes" } else { "❌ no" };
            let uptime = format!("{}s", proc.started_at.elapsed().as_secs());

            let cmd_display: String = proc.command.chars().take(40).collect();
            lines.push(format!(
                "{:<8} {:<6} {:<12} {:<10} {}",
                proc.pid, status, proc.label, uptime, cmd_display
            ));
        }

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: lines.join("\n"),
            is_error: false,
            data: Some(json!({ "count": registry.len() })),
        })
    }

    // ── process_output: read output from a background process ──

    async fn exec_process_output(&self, call: &ToolCall) -> Result<ToolResult> {
        let pid =
            call.arguments["pid"]
                .as_u64()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "process_output".into(),
                    reason: "missing or invalid 'pid' argument".into(),
                })? as u32;
        let max_lines = call.arguments["lines"].as_u64().unwrap_or(50) as usize;

        let registry = PROCESS_REGISTRY.lock().await;
        let proc = registry.get(&pid).ok_or_else(|| claw_core::ClawError::ToolExecution {
            tool: "process_output".into(),
            reason: format!("PID {} not found in tracked processes. Use `process_list` to see tracked processes.", pid),
        })?;

        let log_file = proc.log_file.clone();
        let alive = is_process_alive(pid);
        let label = proc.label.clone();
        let uptime = proc.started_at.elapsed().as_secs();
        drop(registry);

        // Read the log file
        let content = match tokio::fs::read_to_string(&log_file).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Process '{}' (PID {}) — could not read log: {}",
                        label, pid, e
                    ),
                    is_error: true,
                    data: None,
                });
            }
        };

        // Take last N lines
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();
        let tail: Vec<&str> = if total_lines > max_lines {
            all_lines[total_lines - max_lines..].to_vec()
        } else {
            all_lines
        };

        let status_str = if alive { "RUNNING ✅" } else { "EXITED ❌" };
        let header = format!(
            "Process '{}' (PID {}) — {} — uptime {}s — {} total lines\n{}",
            label,
            pid,
            status_str,
            uptime,
            total_lines,
            "─".repeat(60)
        );

        let output = format!("{}\n{}", header, tail.join("\n"));

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: output.chars().take(10_000).collect(),
            is_error: false,
            data: Some(json!({
                "pid": pid,
                "alive": alive,
                "total_lines": total_lines,
                "lines_shown": tail.len(),
            })),
        })
    }

    // ── process_kill: kill a background process ────────────────

    async fn exec_process_kill(&self, call: &ToolCall) -> Result<ToolResult> {
        let pid =
            call.arguments["pid"]
                .as_u64()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "process_kill".into(),
                    reason: "missing or invalid 'pid' argument".into(),
                })?;

        info!(pid = pid, "killing process");

        let output = tokio::process::Command::new("kill")
            .arg(pid.to_string())
            .output()
            .await
            .map_err(|e| claw_core::ClawError::ToolExecution {
                tool: "process_kill".into(),
                reason: e.to_string(),
            })?;

        // Remove from registry
        let label = {
            let mut registry = PROCESS_REGISTRY.lock().await;
            registry
                .remove(&(pid as u32))
                .map(|p| p.label)
                .unwrap_or_default()
        };

        if output.status.success() {
            Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Successfully killed process '{}' (PID {})", label, pid),
                is_error: false,
                data: None,
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to kill process {}: {}", pid, stderr.trim()),
                is_error: true,
                data: None,
            })
        }
    }

    async fn exec_apply_patch(&self, call: &ToolCall) -> Result<ToolResult> {
        let edits = call.arguments["edits"].as_array().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "apply_patch".into(),
                reason: "missing 'edits' array argument".into(),
            }
        })?;

        if edits.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: "No edits provided.".into(),
                is_error: false,
                data: None,
            });
        }

        let mut results: Vec<String> = Vec::new();
        let mut success_count = 0;
        let mut fail_count = 0;

        for (i, edit) in edits.iter().enumerate() {
            let path = match edit["path"].as_str() {
                Some(p) => p,
                None => {
                    fail_count += 1;
                    results.push(format!("Edit {}: FAILED — missing 'path'", i + 1));
                    continue;
                }
            };
            let old_string = match edit["old_string"].as_str() {
                Some(s) => s,
                None => {
                    fail_count += 1;
                    results.push(format!(
                        "Edit {} ({}): FAILED — missing 'old_string'",
                        i + 1,
                        path
                    ));
                    continue;
                }
            };
            let new_string = match edit["new_string"].as_str() {
                Some(s) => s,
                None => {
                    fail_count += 1;
                    results.push(format!(
                        "Edit {} ({}): FAILED — missing 'new_string'",
                        i + 1,
                        path
                    ));
                    continue;
                }
            };

            // Read the file
            let content = match tokio::fs::read_to_string(path).await {
                Ok(c) => c,
                Err(e) => {
                    fail_count += 1;
                    results.push(format!("Edit {} ({}): FAILED — {}", i + 1, path, e));
                    continue;
                }
            };

            // Find and replace
            let occurrences = content.matches(old_string).count();
            if occurrences == 0 {
                fail_count += 1;
                results.push(format!(
                    "Edit {} ({}): FAILED — old_string not found in file",
                    i + 1,
                    path
                ));
                continue;
            }

            let new_content = content.replacen(old_string, new_string, 1);

            // Write back
            match tokio::fs::write(path, &new_content).await {
                Ok(_) => {
                    success_count += 1;
                    let note = if occurrences > 1 {
                        format!(" (replaced 1 of {} occurrences)", occurrences)
                    } else {
                        String::new()
                    };
                    results.push(format!("Edit {} ({}): OK{}", i + 1, path, note));
                }
                Err(e) => {
                    fail_count += 1;
                    results.push(format!(
                        "Edit {} ({}): FAILED — write error: {}",
                        i + 1,
                        path,
                        e
                    ));
                }
            }
        }

        let summary = format!(
            "Applied {}/{} edits ({} failed)\n\n{}",
            success_count,
            edits.len(),
            fail_count,
            results.join("\n")
        );

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            content: summary,
            is_error: fail_count > 0,
            data: None,
        })
    }

    // ── Terminal (PTY) Tool Handlers ──────────────────────────────────

    async fn exec_terminal_open(&self, call: &ToolCall) -> Result<ToolResult> {
        let label = call.arguments["label"].as_str().unwrap_or("default");
        let working_dir = call.arguments["working_dir"].as_str();

        match crate::terminal::terminal_open(label, working_dir).await {
            Ok((id, initial_output)) => {
                let mut content = format!("Terminal {id} opened (label: '{label}')\n");
                if !initial_output.trim().is_empty() {
                    content.push_str(&format!("\nInitial output:\n{initial_output}"));
                }
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    content,
                    is_error: false,
                    data: Some(json!({ "terminal_id": id })),
                })
            }
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to open terminal: {e}"),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_terminal_run(&self, call: &ToolCall) -> Result<ToolResult> {
        let terminal_id = call.arguments["terminal_id"].as_u64().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "terminal_run".into(),
                reason: "missing 'terminal_id' argument".into(),
            }
        })? as u32;

        let command = call.arguments["command"].as_str().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "terminal_run".into(),
                reason: "missing 'command' argument".into(),
            }
        })?;

        let timeout_secs = call.arguments["timeout_secs"].as_u64().unwrap_or(120);
        let timeout_ms = timeout_secs * 1000;

        info!(terminal_id = terminal_id, command = command, "terminal_run");

        match crate::terminal::terminal_run(terminal_id, command, timeout_ms).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: output,
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("terminal_run error: {e}"),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_terminal_view(&self, call: &ToolCall) -> Result<ToolResult> {
        let terminal_id = call.arguments["terminal_id"].as_u64().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "terminal_view".into(),
                reason: "missing 'terminal_id' argument".into(),
            }
        })? as u32;

        let lines = call.arguments["lines"].as_u64().unwrap_or(50) as usize;

        match crate::terminal::terminal_view(terminal_id, lines).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: output,
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("terminal_view error: {e}"),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_terminal_input(&self, call: &ToolCall) -> Result<ToolResult> {
        let terminal_id = call.arguments["terminal_id"].as_u64().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "terminal_input".into(),
                reason: "missing 'terminal_id' argument".into(),
            }
        })? as u32;

        let text =
            call.arguments["text"]
                .as_str()
                .ok_or_else(|| claw_core::ClawError::ToolExecution {
                    tool: "terminal_input".into(),
                    reason: "missing 'text' argument".into(),
                })?;

        let timeout_secs = call.arguments["timeout_secs"].as_u64().unwrap_or(10);
        let timeout_ms = timeout_secs * 1000;

        info!(
            terminal_id = terminal_id,
            text_len = text.len(),
            "terminal_input"
        );

        match crate::terminal::terminal_input(terminal_id, text, timeout_ms).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: output,
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("terminal_input error: {e}"),
                is_error: true,
                data: None,
            }),
        }
    }

    async fn exec_terminal_close(&self, call: &ToolCall) -> Result<ToolResult> {
        let terminal_id = call.arguments["terminal_id"].as_u64().ok_or_else(|| {
            claw_core::ClawError::ToolExecution {
                tool: "terminal_close".into(),
                reason: "missing 'terminal_id' argument".into(),
            }
        })? as u32;

        match crate::terminal::terminal_close(terminal_id).await {
            Ok(msg) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: msg,
                is_error: false,
                data: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("terminal_close error: {e}"),
                is_error: true,
                data: None,
            }),
        }
    }
}

/// Shell-escape a string for safe use in sh -c commands.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Check if a process is still alive by sending signal 0.
fn is_process_alive(pid: u32) -> bool {
    // Use kill -0 which checks process existence without sending a real signal
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
