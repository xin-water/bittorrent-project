use std::{fmt, io};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

// 直接拓展udp,不使用面向接口对象了。
pub(crate) struct UtSocket(UdpSocket,SocketAddr);
impl UtSocket{

    pub fn new(inner: UdpSocket) -> io::Result<Self> {
        let local_addr = inner.local_addr()?;
        Ok(Self(inner, local_addr))
    }

    pub(crate) async fn send(&self, bytes: &[u8], addr: SocketAddr) -> Result<(),&str> {

        let mut bytes_sent = 0;

        while bytes_sent != bytes.len() {
            if let Ok(num_sent) = self.0.send_to(&bytes[bytes_sent..], addr).await {
                bytes_sent += num_sent;
            } else {
                // TODO: Maybe shut down in this case, will fail on every write...
                log::warn!(
                "bittorrent-protocol_dht: DhtSocket Outgoing messenger failed to write {} bytes to {}; {} bytes written \
                   before error...",
                bytes.len(),
                addr,
                bytes_sent
            );
                return Err("DhtSocket data send fail");
            }
        }
        Ok(())
    }


    pub(crate) async fn recv(&self) -> Result<(Vec<u8>, SocketAddr),&str> {
        let mut buffer = vec![0u8; 1500];
        if let Ok((size, addr)) = self.0.recv_from(&mut buffer).await{
            buffer.truncate(size);
            return Ok((buffer, addr));
        }
        Err("DhtSocket data recv error")
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.1
    }

    pub fn ip_version(&self) -> IpVersion {
        match self.1 {
            SocketAddr::V4(_) => IpVersion::V4,
            SocketAddr::V6(_) => IpVersion::V6,
        }
    }

}


#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum IpVersion {
    V4,
    V6,
}

impl fmt::Display for IpVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::V4 => write!(f, "IPv4"),
            Self::V6 => write!(f, "IPv6"),
        }
    }
}