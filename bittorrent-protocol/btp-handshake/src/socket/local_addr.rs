use std::io;
use std::net::SocketAddr;

/// Trait for getting the local address.
pub trait LocalAddr {
    /// Get the local address.
    fn local_addr(&self) -> io::Result<SocketAddr>;
}
