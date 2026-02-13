//! # claw-config
//!
//! Configuration system for the Claw runtime. Reads from `claw.toml`, environment
//! variables, and CLI overrides — in that precedence order.
//!
//! Supports hot-reload via filesystem watcher.

pub mod schema;
pub mod loader;

pub use schema::ClawConfig;
pub use schema::{ConfigWarning, WarningSeverity, ServicesConfig, CredentialsConfig, resolve_context_window};
pub use loader::ConfigLoader;
