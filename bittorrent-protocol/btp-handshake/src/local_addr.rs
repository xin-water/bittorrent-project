use std::io;
use std::net::SocketAddr;

use std::net::TcpStream;

/// Trait for getting the local address.
pub trait LocalAddr {
    /// Get the local address.
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

impl LocalAddr for TcpStream {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        TcpStream::local_addr(self)
    }
}
