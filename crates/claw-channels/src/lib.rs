//! # claw-channels
//!
//! Channel adapter system. Each adapter bridges a messaging platform
//! (Telegram, Discord, WhatsApp, Slack, Signal, etc.) to the Claw agent runtime.
//!
//! Adapters implement the `Channel` trait and are registered with the runtime.
//!
//! ## Supported channels
//!
//! | Channel   | Status           | Setup method                        |
//! |-----------|------------------|-------------------------------------|
//! | WebChat   | ✅ Production    | Built-in, always available          |
//! | Telegram  | ✅ Production    | Bot token from @BotFather           |
//! | WhatsApp  | 🔧 QR Pairing   | `claw channels login whatsapp`      |
//! | Discord   | 🚧 In progress  | Bot token from Discord Developer    |
//! | Slack     | 🚧 In progress  | Bot + App tokens from Slack API     |
//! | Signal    | 🚧 In progress  | `signal-cli` + phone registration   |
//!
//! ## Quick start
//!
//! Run the setup wizard for guided channel configuration:
//! ```bash
//! claw setup
//! ```
//!
//! Or manage channels individually:
//! ```bash
//! claw channels login whatsapp    # Scan QR to link WhatsApp
//! claw channels status            # Show all channel statuses
//! claw channels logout whatsapp   # Unlink WhatsApp
//! ```

pub mod adapter;
pub mod discord;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod webchat;
pub mod whatsapp;

pub use adapter::{Channel, ChannelEvent, IncomingMessage, OutgoingMessage};
