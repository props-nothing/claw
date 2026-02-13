//! # claw-config
//!
//! Configuration system for the Claw runtime. Reads from `claw.toml`, environment
//! variables, and CLI overrides — in that precedence order.
//!
//! Supports hot-reload via filesystem watcher.

pub mod loader;
pub mod schema;

pub use loader::ConfigLoader;
pub use schema::ClawConfig;
pub use schema::{
    ConfigWarning, CredentialsConfig, ServicesConfig, WarningSeverity, resolve_context_window,
};
