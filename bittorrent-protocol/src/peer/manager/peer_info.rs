use std::hash::Hash;
use std::hash::Hasher;
use std::net::SocketAddr;

use crate::util::bt::{InfoHash, PeerId};

/// Information that uniquely identifies a peer.
#[derive(Eq, Debug, Copy, Clone)]
pub struct PeerInfo {
    addr: SocketAddr,
    pid: PeerId,
    hash: InfoHash,
}

impl PeerInfo {
    /// Create a new `PeerInfo` object.
    pub fn new(addr: SocketAddr, pid: PeerId, hash: InfoHash) -> PeerInfo {
        PeerInfo {
            addr: addr,
            pid: pid,
            hash: hash,
        }
    }

    /// Retrieve the peer address.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// Retrieve the peer id.
    pub fn peer_id(&self) -> &PeerId {
        &self.pid
    }

    /// Retrieve the peer info hash.
    pub fn hash(&self) -> &InfoHash {
        &self.hash
    }
}

impl PartialEq for PeerInfo {
    fn eq(&self, other: &PeerInfo) -> bool {
        self.addr.eq(&other.addr) && self.pid.eq(&other.pid) && self.hash.eq(&other.hash)
    }
}

impl Hash for PeerInfo {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.addr.hash(state);
        self.pid.hash(state);
        self.hash.hash(state);
    }
}
