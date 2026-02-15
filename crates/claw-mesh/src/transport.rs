//! libp2p swarm transport layer for the Claw mesh network.
//!
//! This module is only compiled when the `p2p` feature is enabled.
//! It provides the real networking: TCP transport with Noise encryption,
//! Yamux multiplexing, GossipSub pub/sub, mDNS local discovery,
//! Identify protocol, and Kademlia DHT.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, Swarm, gossipsub, identify, kad, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::protocol::MeshMessage;

/// GossipSub topic for all Claw mesh messages.
pub const MESH_TOPIC: &str = "claw/mesh/v1";

/// Commands sent from the MeshNode API to the background swarm event loop.
#[derive(Debug)]
pub enum SwarmCommand {
    /// Publish a serialized message to the gossipsub topic.
    Publish(Vec<u8>),
    /// Dial a peer at a specific multiaddr.
    Dial(Multiaddr),
    /// Shut down the swarm.
    Shutdown,
}

/// Combined network behaviour for the Claw mesh.
///
/// Composes four sub-protocols:
/// - **GossipSub** — pub/sub broadcast messaging for all mesh communication
/// - **mDNS** — zero-config local peer discovery on the same LAN
/// - **Identify** — exchange peer metadata (agent version, capabilities)
/// - **Kademlia** — distributed hash table for WAN peer routing
#[derive(NetworkBehaviour)]
pub struct ClawBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
}

/// Handle returned after starting the swarm — provides the local peer ID
/// and a command channel for controlling the swarm from the MeshNode API.
pub struct SwarmHandle {
    pub peer_id: PeerId,
    pub command_tx: mpsc::Sender<SwarmCommand>,
}

/// Build and start the libp2p swarm.
///
/// This spawns a background tokio task running the swarm event loop.
/// Incoming mesh messages are forwarded to `message_tx`.  The caller
/// controls the swarm via the returned `SwarmHandle.command_tx`.
pub async fn start_swarm(
    listen_addr: &str,
    bootstrap_peers: &[String],
    capabilities: Vec<String>,
    message_tx: mpsc::Sender<MeshMessage>,
) -> claw_core::Result<SwarmHandle> {
    let mut swarm = build_swarm(capabilities.clone())?;

    let peer_id = *swarm.local_peer_id();
    info!(peer_id = %peer_id, "libp2p swarm identity created");

    // Listen on the configured multiaddr
    let addr: Multiaddr = listen_addr.parse().map_err(|e| {
        claw_core::ClawError::Agent(format!("invalid mesh listen address '{listen_addr}': {e}"))
    })?;
    swarm
        .listen_on(addr)
        .map_err(|e| claw_core::ClawError::Agent(format!("failed to listen: {e}")))?;

    // Dial bootstrap peers
    for peer_addr in bootstrap_peers {
        match peer_addr.parse::<Multiaddr>() {
            Ok(addr) => {
                info!(addr = %addr, "dialing bootstrap peer");
                if let Err(e) = swarm.dial(addr) {
                    warn!(error = %e, "failed to dial bootstrap peer");
                }
            }
            Err(e) => {
                warn!(addr = %peer_addr, error = %e, "invalid bootstrap peer address, skipping");
            }
        }
    }

    // Subscribe to the mesh GossipSub topic
    let topic = gossipsub::IdentTopic::new(MESH_TOPIC);
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .map_err(|e| {
            claw_core::ClawError::Agent(format!("failed to subscribe to gossipsub topic: {e}"))
        })?;

    // Create the command channel (MeshNode → swarm task)
    let (command_tx, command_rx) = mpsc::channel(256);

    // Spawn the swarm event loop as a background task
    let peer_id_for_announce = peer_id;
    tokio::spawn(run_swarm_loop(
        swarm,
        topic,
        message_tx,
        command_rx,
        peer_id_for_announce,
        capabilities,
    ));

    Ok(SwarmHandle {
        peer_id,
        command_tx,
    })
}

/// Build the libp2p Swarm with all sub-protocols configured.
fn build_swarm(capabilities: Vec<String>) -> claw_core::Result<Swarm<ClawBehaviour>> {
    build_swarm_inner(capabilities)
        .map_err(|e| claw_core::ClawError::Agent(format!("failed to build libp2p swarm: {e}")))
}

/// Inner builder that uses `Box<dyn Error>` for ergonomic `?` propagation
/// across the various libp2p builder error types.
fn build_swarm_inner(
    capabilities: Vec<String>,
) -> std::result::Result<Swarm<ClawBehaviour>, Box<dyn std::error::Error + Send + Sync>> {
    let swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            // ── GossipSub ──────────────────────────────────────────
            // Content-address messages so duplicates are suppressed.
            let message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(10))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(message_id_fn)
                .max_transmit_size(256 * 1024) // 256 KB max message size
                .build()
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;

            // ── mDNS ───────────────────────────────────────────────
            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;

            // ── Identify ───────────────────────────────────────────
            // Encode capabilities into the agent_version string so
            // peers learn about each other's abilities on connect.
            // Format: "claw/<version>;<cap1>,<cap2>,<cap3>;<hostname>;<os>"
            let hostname = std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("COMPUTERNAME"))
                .unwrap_or_else(|_| {
                    std::process::Command::new("hostname")
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "unknown".into())
                });
            let agent_version = format!(
                "claw/{};{};{};{}",
                env!("CARGO_PKG_VERSION"),
                capabilities.join(","),
                hostname,
                std::env::consts::OS,
            );
            let identify = identify::Behaviour::new(
                identify::Config::new("/claw/mesh/1.0.0".to_string(), key.public())
                    .with_agent_version(agent_version),
            );

            // ── Kademlia ───────────────────────────────────────────
            let peer_id = key.public().to_peer_id();
            let kademlia = kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id));

            Ok(ClawBehaviour {
                gossipsub,
                mdns,
                identify,
                kademlia,
            })
        })?
        .build();

    Ok(swarm)
}

/// The main swarm event loop — runs as a background tokio task.
///
/// Dispatches on three event sources:
/// 1. Swarm events (mDNS discovery, gossipsub messages, identify, etc.)
/// 2. Commands from the MeshNode API (publish, dial, shutdown)
/// 3. Periodic announce timer (broadcast our presence every 30s)
async fn run_swarm_loop(
    mut swarm: Swarm<ClawBehaviour>,
    topic: gossipsub::IdentTopic,
    message_tx: mpsc::Sender<MeshMessage>,
    mut command_rx: mpsc::Receiver<SwarmCommand>,
    local_peer_id: PeerId,
    capabilities: Vec<String>,
) {
    let mut announce_interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            // ── Swarm events ───────────────────────────────────────
            event = swarm.select_next_some() => {
                handle_swarm_event(
                    &mut swarm,
                    &topic,
                    &message_tx,
                    event,
                    &local_peer_id,
                    &capabilities,
                ).await;
            }

            // ── Commands from MeshNode API ─────────────────────────
            Some(cmd) = command_rx.recv() => {
                match cmd {
                    SwarmCommand::Publish(data) => {
                        if let Err(e) = swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic.clone(), data)
                        {
                            warn!(error = %e, "failed to publish gossipsub message");
                        }
                    }
                    SwarmCommand::Dial(addr) => {
                        if let Err(e) = swarm.dial(addr) {
                            warn!(error = %e, "failed to dial peer");
                        }
                    }
                    SwarmCommand::Shutdown => {
                        info!("mesh swarm shutting down");
                        break;
                    }
                }
            }

            // ── Periodic self-announce ─────────────────────────────
            _ = announce_interval.tick() => {
                let hostname = std::env::var("HOSTNAME")
                    .or_else(|_| std::env::var("COMPUTERNAME"))
                    .unwrap_or_else(|_| "unknown".into());
                let announce = MeshMessage::Announce {
                    peer_id: local_peer_id.to_string(),
                    hostname,
                    capabilities: capabilities.clone(),
                    os: std::env::consts::OS.to_string(),
                };
                if let Ok(data) = serde_json::to_vec(&announce)
                    && let Err(e) = swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(topic.clone(), data)
                    {
                        // PublishError::InsufficientPeers is normal when alone
                        debug!(error = %e, "periodic announce publish failed (may be alone on network)");
                    }
            }
        }
    }
}

/// Handle a single swarm event.
async fn handle_swarm_event(
    swarm: &mut Swarm<ClawBehaviour>,
    topic: &gossipsub::IdentTopic,
    message_tx: &mpsc::Sender<MeshMessage>,
    event: SwarmEvent<ClawBehaviourEvent>,
    local_peer_id: &PeerId,
    capabilities: &[String],
) {
    match event {
        // ── mDNS: discovered new peers on the LAN ──────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
            for (peer_id, addr) in list {
                info!(peer = %peer_id, addr = %addr, "mDNS discovered peer");
                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr.clone());

                // Register the peer immediately with basic info so it shows
                // up in `claw mesh peers` right away.  The Identify exchange
                // will update capabilities/hostname later.
                let peer_hostname = addr
                    .iter()
                    .find_map(|p| match p {
                        libp2p::multiaddr::Protocol::Ip4(ip) => Some(ip.to_string()),
                        libp2p::multiaddr::Protocol::Dns(name) => Some(name.to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let announce = MeshMessage::Announce {
                    peer_id: peer_id.to_string(),
                    hostname: peer_hostname,
                    capabilities: vec![],
                    os: "unknown".to_string(),
                };
                let _ = message_tx.send(announce).await;

                // Dial the peer to establish a connection — this triggers the
                // Identify exchange which fills in capabilities and hostname.
                if let Err(e) = swarm.dial(addr) {
                    warn!(peer = %peer_id, error = %e, "dial after mDNS discovery failed (may already be connected)");
                }
            }
        }

        // ── mDNS: peer expired ─────────────────────────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
            for (peer_id, _addr) in list {
                info!(peer = %peer_id, "mDNS peer expired");
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
            }
        }

        // ── GossipSub: received a message ──────────────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        })) => {
            debug!(
                source = %propagation_source,
                id = %message_id,
                bytes = message.data.len(),
                "received gossipsub message"
            );
            match serde_json::from_slice::<MeshMessage>(&message.data) {
                Ok(mesh_msg) => {
                    if message_tx.send(mesh_msg).await.is_err() {
                        warn!("mesh message receiver dropped, stopping");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to deserialize mesh message");
                }
            }
        }

        // ── GossipSub: subscription change ─────────────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed {
            peer_id,
            topic,
        })) => {
            debug!(peer = %peer_id, topic = %topic, "peer subscribed to topic");
        }

        // ── Identify: received peer info ───────────────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Identify(identify::Event::Received {
            peer_id,
            info,
            ..
        })) => {
            info!(
                peer = %peer_id,
                agent = %info.agent_version,
                "identified peer"
            );

            // Parse capabilities from agent_version:
            // Format: "claw/<version>;<cap1>,<cap2>;<hostname>;<os>"
            if info.agent_version.starts_with("claw/") {
                let parts: Vec<&str> = info.agent_version.splitn(4, ';').collect();
                let capabilities: Vec<String> = parts
                    .get(1)
                    .unwrap_or(&"")
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                let hostname = parts.get(2).unwrap_or(&"unknown").to_string();
                let os = parts.get(3).unwrap_or(&"unknown").to_string();

                // Forward an Announce to the runtime so it can register the peer
                let announce = MeshMessage::Announce {
                    peer_id: peer_id.to_string(),
                    hostname,
                    capabilities,
                    os,
                };
                let _ = message_tx.send(announce).await;
            }

            // Add Kademlia addresses for the identified peer
            for addr in info.listen_addrs {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
            }
        }

        // ── Kademlia events ────────────────────────────────────
        SwarmEvent::Behaviour(ClawBehaviourEvent::Kademlia(event)) => {
            debug!(event = ?event, "kademlia event");
        }

        // ── Connection lifecycle ───────────────────────────────
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(addr = %address, "mesh node listening");
        }
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            info!(peer = %peer_id, "mesh connection established");

            // Publish our own Announce via GossipSub so the remote peer
            // learns our capabilities (don't rely solely on Identify).
            let hostname = std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("COMPUTERNAME"))
                .unwrap_or_else(|_| {
                    std::process::Command::new("hostname")
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "unknown".into())
                });
            let announce = MeshMessage::Announce {
                peer_id: local_peer_id.to_string(),
                hostname,
                capabilities: capabilities.to_vec(),
                os: std::env::consts::OS.to_string(),
            };
            if let Ok(data) = serde_json::to_vec(&announce)
                && let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                    debug!(error = %e, "connection announce publish failed");
                }
        }
        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
            info!(peer = %peer_id, cause = ?cause, "mesh connection closed");
            // Notify the runtime to remove the disconnected peer
            let _ = message_tx
                .send(MeshMessage::PeerLeft {
                    peer_id: peer_id.to_string(),
                })
                .await;
        }
        SwarmEvent::IncomingConnection {
            local_addr,
            send_back_addr,
            ..
        } => {
            debug!(local = %local_addr, remote = %send_back_addr, "incoming connection");
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            warn!(peer = ?peer_id, error = %error, "outgoing connection error");
        }
        SwarmEvent::IncomingConnectionError {
            local_addr,
            send_back_addr,
            error,
            ..
        } => {
            warn!(local = %local_addr, remote = %send_back_addr, error = %error, "incoming connection error");
        }

        _ => {}
    }
}
