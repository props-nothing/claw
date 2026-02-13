//! Peer discovery strategies for the Claw mesh network.
//!
//! mDNS discovery is handled automatically by the libp2p swarm
//! (see `transport.rs`).  The types here describe the available strategies
//! and provide helpers for non-swarm use cases.

use tracing::info;

/// Discovery methods for finding peers.
#[derive(Debug, Clone)]
pub enum DiscoveryMethod {
    /// mDNS for LAN discovery (handled by libp2p::mdns).
    Mdns,
    /// Bootstrap peers — known multiaddrs to dial on startup.
    Bootstrap(Vec<String>),
    /// Tailscale-based discovery (future — see Phase 6.4).
    Tailscale,
}

/// Discover peers on the local network via mDNS.
///
/// mDNS runs inside the libp2p swarm and peers are discovered automatically.
/// This function exists for manual / diagnostic use.
pub async fn discover_local() -> Vec<String> {
    info!("discovering local mesh peers via mDNS");
    // mDNS is managed by the swarm — no standalone discovery needed.
    // Peers appear via SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(...)).
    info!("mDNS discovery is managed by the libp2p swarm");
    vec![]
}
