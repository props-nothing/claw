use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a session.
pub type SessionId = Uuid;

/// Unique identifier for a goal.
pub type GoalId = Uuid;

/// Unique identifier for a peer in the mesh network.
pub type PeerId = String;

/// Unique identifier for a plugin.
pub type PluginId = String;

/// Unique identifier for a channel.
pub type ChannelId = String;

/// A device in the mesh network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub peer_id: PeerId,
    pub hostname: String,
    pub os: Os,
    pub arch: Arch,
    /// What this device can do (camera, gpu, browser, shell, etc.)
    pub capabilities: Vec<String>,
    /// Whether this device can run local model inference.
    pub local_inference: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    MacOS,
    Windows,
    Android,
    IOS,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86_64,
    X86,
    Aarch64,
    Arm,
    Wasm32,
}

impl Os {
    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Os::Linux
        }
        #[cfg(target_os = "macos")]
        {
            Os::MacOS
        }
        #[cfg(target_os = "windows")]
        {
            Os::Windows
        }
        #[cfg(target_os = "android")]
        {
            Os::Android
        }
        #[cfg(target_os = "ios")]
        {
            Os::IOS
        }
    }
}

impl Arch {
    pub fn current() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Arch::X86_64
        }
        #[cfg(target_arch = "x86")]
        {
            Arch::X86
        }
        #[cfg(target_arch = "aarch64")]
        {
            Arch::Aarch64
        }
        #[cfg(target_arch = "arm")]
        {
            Arch::Arm
        }
        #[cfg(target_arch = "wasm32")]
        {
            Arch::Wasm32
        }
    }
}

/// A capability token granted by the user to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// What the capability grants, e.g. "fs.read", "fs.write", "shell.exec", "network.http".
    pub name: String,
    /// Optional scope constraint, e.g. a path glob for fs capabilities.
    #[serde(default)]
    pub scope: Option<String>,
    /// When this capability expires (None = permanent until revoked).
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
