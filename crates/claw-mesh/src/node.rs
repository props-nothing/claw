use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::protocol::MeshMessage;

/// Known peer information tracked by the local node.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub hostname: String,
    pub capabilities: Vec<String>,
    pub os: String,
    #[serde(skip)]
    pub last_seen: std::time::Instant,
}

/// A node in the Claw mesh network.
///
/// With the `p2p` feature enabled, this wraps a full libp2p swarm with:
/// - TCP transport + Noise encryption + Yamux multiplexing
/// - GossipSub for pub/sub broadcast messaging
/// - mDNS for local peer discovery
/// - Kademlia for distributed routing and DHT
/// - Identify protocol for exchanging peer metadata
///
/// Without the `p2p` feature, this is an in-memory stub that tracks peers
/// but cannot actually communicate over the network.
pub struct MeshNode {
    peer_id: String,
    /// Known peers and their capabilities.
    peers: HashMap<String, PeerInfo>,
    /// Channel for forwarding mesh messages into the runtime.
    message_tx: Option<mpsc::Sender<MeshMessage>>,
    /// Whether the node is currently running.
    running: bool,
    /// Command sender for the libp2p swarm.
    command_tx: Option<mpsc::Sender<crate::transport::SwarmCommand>>,
}

impl MeshNode {
    /// Create a new mesh node with a random identity.
    pub fn new() -> claw_core::Result<Self> {
        // With the p2p feature, the real PeerId is assigned when the swarm starts.
        // We use a UUID placeholder here so the rest of the codebase can reference
        // the node before start() is called.
        let peer_id = uuid::Uuid::new_v4().to_string();
        info!(peer_id = %peer_id, "mesh node identity created");

        Ok(Self {
            peer_id,
            peers: HashMap::new(),
            message_tx: None,
            running: false,
            command_tx: None,
        })
    }

    /// Get our peer ID (libp2p PeerId string when p2p is enabled, UUID otherwise).
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// Whether the node is currently running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get known peers.
    pub fn peers(&self) -> &HashMap<String, PeerInfo> {
        &self.peers
    }

    /// Get a snapshot of peers as a Vec (for serialization / API responses).
    pub fn peer_list(&self) -> Vec<&PeerInfo> {
        self.peers.values().collect()
    }

    /// Find a peer with a specific capability.
    pub fn find_peer_with_capability(&self, capability: &str) -> Option<&PeerInfo> {
        self.peers
            .values()
            .find(|p| p.capabilities.iter().any(|c| c == capability))
    }

    /// Find the least-recently-used peer with a specific capability (basic load balancing).
    pub fn find_best_peer_for_capability(&self, capability: &str) -> Option<&PeerInfo> {
        self.peers
            .values()
            .filter(|p| p.capabilities.iter().any(|c| c == capability))
            .min_by_key(|p| p.last_seen)
    }

    /// Register a discovered peer (or update an existing one).
    pub fn register_peer(&mut self, info: PeerInfo) {
        info!(
            peer_id = %info.peer_id,
            hostname = %info.hostname,
            capabilities = ?info.capabilities,
            "registered mesh peer"
        );
        self.peers.insert(info.peer_id.clone(), info);
    }

    /// Remove a peer that has disconnected or expired.
    pub fn remove_peer(&mut self, peer_id: &str) {
        if self.peers.remove(peer_id).is_some() {
            info!(peer_id = %peer_id, "removed mesh peer");
        }
    }

    /// Number of known peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    // ── p2p feature: real libp2p swarm ─────────────────────────────

    /// Start the mesh node with the libp2p swarm.
    ///
    /// Spawns a background task running the swarm event loop.
    /// Returns a receiver for incoming `MeshMessage`s from the network.
    pub async fn start(
        &mut self,
        listen_addr: &str,
        bootstrap_peers: &[String],
        capabilities: Vec<String>,
    ) -> claw_core::Result<mpsc::Receiver<MeshMessage>> {
        let (tx, rx) = mpsc::channel(256);
        self.message_tx = Some(tx.clone());

        let handle =
            crate::transport::start_swarm(listen_addr, bootstrap_peers, capabilities, tx).await?;

        // Replace the placeholder UUID with the real libp2p PeerId
        self.peer_id = handle.peer_id.to_string();
        self.command_tx = Some(handle.command_tx);
        self.running = true;

        info!(
            listen = listen_addr,
            peer_id = %self.peer_id,
            "mesh node started with libp2p swarm"
        );

        Ok(rx)
    }

    /// Broadcast a message to all peers via GossipSub.
    pub async fn broadcast(&self, message: &MeshMessage) -> claw_core::Result<()> {
        let data =
            serde_json::to_vec(message).map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

        if let Some(ref cmd_tx) = self.command_tx {
            cmd_tx
                .send(crate::transport::SwarmCommand::Publish(data))
                .await
                .map_err(|_| claw_core::ClawError::Agent("swarm task not running".into()))?;
            debug!("broadcast mesh message to {} peers", self.peers.len());
        } else {
            warn!("cannot broadcast: mesh node not started");
        }

        Ok(())
    }

    /// Send a message to a specific peer.
    ///
    /// With GossipSub, this publishes to the topic with the target peer_id
    /// encoded in the message. The target peer processes it; others ignore it.
    pub async fn send_to(&self, peer_id: &str, message: &MeshMessage) -> claw_core::Result<()> {
        if !self.peers.contains_key(peer_id) {
            return Err(claw_core::ClawError::PeerUnreachable(peer_id.to_string()));
        }

        let data =
            serde_json::to_vec(message).map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

        if let Some(ref cmd_tx) = self.command_tx {
            cmd_tx
                .send(crate::transport::SwarmCommand::Publish(data))
                .await
                .map_err(|_| claw_core::ClawError::Agent("swarm task not running".into()))?;
            info!(target_peer = peer_id, "sent mesh message via gossipsub");
        } else {
            warn!("cannot send: mesh node not started");
        }

        Ok(())
    }

    /// Dial a peer at a specific multiaddr.
    pub async fn dial(&self, addr: &str) -> claw_core::Result<()> {
        let multiaddr: libp2p::Multiaddr = addr
            .parse()
            .map_err(|e| claw_core::ClawError::Agent(format!("invalid multiaddr '{addr}': {e}")))?;

        if let Some(ref cmd_tx) = self.command_tx {
            cmd_tx
                .send(crate::transport::SwarmCommand::Dial(multiaddr))
                .await
                .map_err(|_| claw_core::ClawError::Agent("swarm task not running".into()))?;
        }
        Ok(())
    }

    /// Stop the mesh node.
    pub async fn stop(&mut self) -> claw_core::Result<()> {
        info!("stopping mesh node");

        if let Some(cmd_tx) = self.command_tx.take() {
            let _ = cmd_tx.send(crate::transport::SwarmCommand::Shutdown).await;
        }

        self.message_tx = None;
        self.running = false;
        self.peers.clear();
        Ok(())
    }

    /// Process an incoming MeshMessage — updates local state as needed.
    ///
    /// Returns `true` if the message was handled locally (e.g. an Announce
    /// that updated our peer table), `false` if it should be forwarded to
    /// the runtime for further processing (e.g. a TaskAssign).
    pub fn handle_message(&mut self, message: &MeshMessage) -> bool {
        match message {
            MeshMessage::Announce {
                peer_id,
                hostname,
                capabilities,
                os,
            } => {
                // Don't register ourselves
                if peer_id == &self.peer_id {
                    return true;
                }
                self.register_peer(PeerInfo {
                    peer_id: peer_id.clone(),
                    hostname: hostname.clone(),
                    capabilities: capabilities.clone(),
                    os: os.clone(),
                    last_seen: std::time::Instant::now(),
                });
                true // handled
            }
            MeshMessage::Ping { peer_id, .. } => {
                if let Some(peer) = self.peers.get_mut(peer_id) {
                    peer.last_seen = std::time::Instant::now();
                }
                true
            }
            MeshMessage::Pong { peer_id, .. } => {
                if let Some(peer) = self.peers.get_mut(peer_id) {
                    peer.last_seen = std::time::Instant::now();
                }
                true
            }
            // TaskAssign and TaskResult need runtime processing
            MeshMessage::TaskAssign(_) | MeshMessage::TaskResult { .. } => false,
            // SyncDelta needs runtime processing
            MeshMessage::SyncDelta { .. } => false,
            // DirectMessage needs runtime processing
            MeshMessage::DirectMessage { .. } => false,
            // PeerLeft — remove the peer from our table
            MeshMessage::PeerLeft { peer_id } => {
                self.remove_peer(peer_id);
                true
            }
        }
    }
}
