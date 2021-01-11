use crate::util::bt::{InfoHash, PeerId};
use std::net::{SocketAddr, TcpStream};

pub enum Task<T:Send> {
    Connect(Option<PeerId>, InfoHash, SocketAddr),
    Metadata(T),
    Shutdown,
}

/// Bittorrent TCP peer that has been handshaken.
#[derive(Debug)]
pub struct BTPeer {
    stream: TcpStream,
    pid: PeerId,
    hash: InfoHash,
}

impl BTPeer {
    /// Create a new BTPeer container.
    pub fn new(stream: TcpStream, hash: InfoHash, pid: PeerId) -> BTPeer {
        BTPeer {
            stream: stream,
            hash: hash,
            pid: pid,
        }
    }

    /// Destroy the BTPeer container and return the contained objects.
    pub fn destory(self) -> (TcpStream, InfoHash, PeerId) {
        (self.stream, self.hash, self.pid)
    }
}
