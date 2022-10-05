use std::{fmt, io};
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use async_trait::async_trait;

pub struct Socket( Box<dyn SocketTrait + Send + Sync + 'static>, SocketAddr);

impl Socket{

    pub fn new<S: SocketTrait + Send + Sync + 'static>(inner: S) -> io::Result<Self> {
        let inner = Box::new(inner);
        let local_addr = inner.local_addr()?;
        Ok(Self(inner, local_addr))
    }

    pub(crate) async fn send(&self, bytes: &[u8], addr: SocketAddr) -> io::Result<()> {
        // Note: if the socket fails to send the entire buffer, then there is no point in trying to
        // send the rest (no node will attempt to reassemble two or more datagrams into a
        // meaningful message).
        self.0.send_to(&bytes, &addr).await
    }


    pub(crate) async fn recv(&mut self) -> io::Result<(Vec<u8>, SocketAddr)> {
        let mut buffer = vec![0u8; 1500];
        let (size, addr) = self.0.recv_from(&mut buffer).await.unwrap();
        buffer.truncate(size);
        Ok((buffer, addr))
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

#[async_trait]
pub trait SocketTrait {
    async fn send_to(&self, buf: &[u8], target: &SocketAddr) -> io::Result<()>;
    async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

#[async_trait]
impl SocketTrait for UdpSocket{
   async fn send_to(&self, buf: &[u8], target: &SocketAddr) -> io::Result<()> {

        let mut bytes_sent = 0;

        while bytes_sent != buf.len() {
            if let Ok(num_sent) = self.send_to(&buf[bytes_sent..], target).await {
                bytes_sent += num_sent;
            } else {
                // TODO: Maybe shut down in this case, will fail on every write...
                warn!(
                "bittorrent-protocol_dht: Socket Outgoing messenger failed to write {} bytes to {}; {} bytes written \
                   before error...",
                buf.len(),
                target,
                bytes_sent
            );
                break;
            }
        }
        Ok(())
    }

   async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.recv_from(buf).await
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
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




