//! # claw-cli
//!
//! Command-line interface for the Claw agent runtime.
//!
//! ## Commands
//!
//! - `claw start` — Start the agent runtime
//! - `claw chat` — Interactive chat in the terminal
//! - `claw status` — Show runtime status
//! - `claw config` — Show/edit configuration
//! - `claw plugin` — Manage plugins
//! - `claw doctor` — Audit configuration for security issues

pub mod commands;

pub use commands::Cli;
