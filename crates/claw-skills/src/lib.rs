//! # claw-skills
//!
//! Skills are prompt-injected instructions that teach the LLM how to use its
//! existing tools for specific workflows. Each skill is a directory containing
//! a `SKILL.md` file (Markdown with YAML frontmatter).
//!
//! Unlike the old TOML step-executor approach, skills are NOT executed by the
//! runtime. Instead, the LLM reads the SKILL.md instructions and drives the
//! workflow itself using built-in tools. This is more flexible and allows the
//! LLM to adapt, handle errors, and make decisions within the workflow.
//!
//! ## SKILL.md Format
//!
//! ```markdown
//! ---
//! name: server-management
//! description: Manage remote servers via SSH
//! version: 1.0.0
//! tags: [devops, ssh, servers]
//! ---
//!
//! # Server Management
//!
//! ## When to use this skill
//! When the user asks you to manage, configure, or troubleshoot a remote server.
//!
//! ## Instructions
//! 1. First, check connectivity with `shell_exec` running `ssh -o ConnectTimeout=5 user@host echo ok`
//! 2. Use `shell_exec` with ssh commands to run remote operations
//! 3. For file transfers, use `scp` or `rsync` via `shell_exec`
//! ```
//!
//! ## How skills work
//!
//! 1. At startup, the registry discovers SKILL.md files in skill directories
//! 2. Skill names + descriptions are listed in the system prompt
//! 3. When the LLM decides a skill applies, it reads the SKILL.md via `file_read`
//! 4. The LLM follows the instructions using its existing tools
//! 5. No special "skill-*" tools — the LLM uses tools directly

pub mod definition;
pub mod registry;

pub use definition::SkillDefinition;
pub use registry::SkillRegistry;
