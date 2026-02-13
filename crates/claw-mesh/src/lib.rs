//! # claw-mesh
//!
//! Peer-to-peer mesh networking for multi-device agent coordination.
//!
//! Your laptop, phone, and server form a swarm — sharing memory, delegating
//! tasks to the best-suited device, and maintaining consensus via CRDTs.
//!
//! Built on libp2p with Noise encryption, Yamux multiplexing, and GossipSub
//! for pub/sub messaging.
//!
pub mod discovery;
pub mod node;
pub mod protocol;
pub mod transport;

pub use node::{MeshNode, PeerInfo};
pub use protocol::{MeshMessage, TaskAssignment};
