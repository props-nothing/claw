//! # claw-plugin
//!
//! WebAssembly-based plugin host. Plugins (skills) are compiled to WASM and
//! run in sandboxed environments with capability-gated access to the host.
//!
//! ## Plugin Manifest
//!
//! Each plugin ships with a `plugin.toml` manifest:
//!
//! ```toml
//! [plugin]
//! name = "web-search"
//! version = "1.0.0"
//! description = "Search the web using multiple engines"
//! authors = ["Claw Community"]
//!
//! [capabilities]
//! network = ["https://api.search.brave.com/*", "https://www.google.com/*"]
//! ```

pub mod host;
pub mod manifest;
pub mod registry;

pub use host::PluginHost;
pub use manifest::PluginManifest;
pub use registry::PluginRegistry;
