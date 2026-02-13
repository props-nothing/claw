//! # claw-device
//!
//! Universal device control for the Claw AI runtime.
//!
//! Provides the agent with the ability to control:
//! - **Browsers** — via Chrome DevTools Protocol (CDP) over WebSocket
//! - **Android devices** — via ADB (Android Debug Bridge)
//! - **iOS devices** — via libimobiledevice / Xcode command-line tools
//!
//! Each subsystem exposes a high-level async API that the tool layer in
//! `claw-runtime` calls into.  All device interaction is stateful (managed
//! sessions) and guarded by the autonomy / guardrail system.

pub mod android;
pub mod browser;
pub mod ios;
pub mod tools;

pub use android::AndroidBridge;
pub use browser::BrowserManager;
pub use ios::IosBridge;
pub use tools::DeviceTools;
