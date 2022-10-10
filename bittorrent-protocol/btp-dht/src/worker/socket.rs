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
       self.0.recv().await
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
    async fn recv(&mut self) -> io::Result<(Vec<u8>, SocketAddr)>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}



// 将消息读取封装到接口实现中，消除站溢出bug,
// 不明白为什么直接在接口对象上拓展后用selece宏读取会导致站溢出，
// 可能是异步接口宏 tokio-udp 与 select宏的组合兼容问题。
// 考虑不使用trait，想直接对具体类型进行拓展，后期看情况优化。
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

   async fn recv(&mut self) -> io::Result<(Vec<u8>, SocketAddr)> {
       let mut buffer = vec![0u8; 1500];
       let (size, addr) = self.recv_from(&mut buffer).await.unwrap();
       buffer.truncate(size);
       Ok((buffer, addr))
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

// 直接拓展udp,不使用面向接口对象了。
pub(crate) struct DhtSocket(UdpSocket,SocketAddr);
impl DhtSocket{

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
                warn!(
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







