use std::fmt::Debug;
use std::io;

use crate::socket::local_addr::LocalAddr;

use tokio::io::{ AsyncRead, AsyncWrite, Error, ErrorKind};
use tokio::net::{TcpStream, TcpListener};
use std::net::{SocketAddr, Incoming};
use std::option::Option::Some;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures::stream::Stream;
use btp_utp::{UtpSocket, UtpListener, UtpStream};
use async_trait::async_trait;

/// Trait for initializing connections over an abstract `Transport`.
#[async_trait]
pub trait Transport {
    /// Concrete socket.
    type Socket: AsyncRead + AsyncWrite + 'static + Unpin + Debug;

    /// Concrete listener.
    type Listener: Stream<Item = (Self::Socket, SocketAddr) > + LocalAddr + 'static + Send + std::marker::Unpin;

    /// Connect to the given address over this transport, using the supplied `Handle`.
    async fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket>;

    /// Listen to the given address for this transport, using the supplied `Handle`.
    async fn listen(&self, addr: &SocketAddr ) -> io::Result<Self::Listener>;
}

//----------------------------------------------------------------------------------//

/// Defines a `Transport` operating over TCP.
#[derive(Debug)]
pub struct TcpTransport;

#[async_trait]
impl Transport for TcpTransport {
    type Socket = TcpStream;
    type Listener = TcpListenerStream;

    async fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket> {
        TcpStream::connect(addr).await
    }

    async fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
        let listener = TcpListener::bind(addr).await?;
        let listen_addr = listener.local_addr()?;

        Ok(TcpListenerStream::new(listen_addr, listener))
    }
}

/// Convenient object that wraps a listener stream `L`, and also implements `LocalAddr`.
pub struct TcpListenerStream {
    listen_addr: SocketAddr,
    listener: TcpListener,
}

impl TcpListenerStream {

    fn new(listen_addr: SocketAddr, listener: TcpListener) -> TcpListenerStream {
        TcpListenerStream {
            listen_addr: listen_addr,
            listener: listener,
        }
    }
}

impl LocalAddr for TcpListenerStream {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.listen_addr)
    }
}

impl Stream for TcpListenerStream  {
    type Item = (TcpStream,SocketAddr);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match  Pin::new(&self.listener).poll_accept(cx){
            Poll::Pending => { Poll::Pending }
            Poll::Ready(Err(_)) => { Poll::Ready(None) }
            Poll::Ready(Ok(x)) => { Poll::Ready(Some(x)) }
        }
    }
}


/// Defines a `Transport` operating over UTP.
pub struct UtpTransport;

// #[async_trait]
// impl Transport for UtpTransport {
//     type Socket = UtpSocket;
//     type Listener = UtpListenerStream;
//
//     async fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket> {
//         UtpSocket::connect(addr)
//     }
//
//     async fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
//         let listener = UtpListener::bind(addr)?;
//         let listen_addr = listener.local_addr()?;
//
//         Ok(UtpListenerStream::new(listen_addr, listener))
//     }
// }


/// Convenient object that wraps a listener stream `L`, and also implements `LocalAddr`.
pub struct UtpListenerStream {
    listen_addr: SocketAddr,
    listener: UtpListener,
}

impl UtpListenerStream {

    fn new(listen_addr: SocketAddr, listener: UtpListener) -> UtpListenerStream {
        UtpListenerStream {
            listen_addr: listen_addr,
            listener: listener,
        }
    }
}

impl LocalAddr for UtpListenerStream {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.listen_addr)
    }
}

impl Stream for UtpListenerStream  {
    type Item = (UtpSocket,SocketAddr);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Ok((result,addr)) = self.listener.accept() {
            Poll::Ready(Some((result,addr)))
        }else {
            Poll::Ready(None)
        }
    }
}
//----------------------------------------------------------------------------------//

#[cfg(test)]
pub mod test_transports {
    use std::io::{self, Cursor, Error, ErrorKind};
    use std::net::SocketAddr;

    use super::Transport;
    use crate::LocalAddr;
    use crate::stream::Stream;


    pub struct MockTransport;

    impl Transport for MockTransport {
        type Socket = Cursor<Vec<u8>>;
        type Listener = MockListener;

        fn connect(&self, _addr: &SocketAddr) -> io::Result<Self::Socket> {
            Ok(Cursor::new(Vec::new()))
        }

        fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
            Ok(MockListener::new(*addr))
        }
    }

    //----------------------------------------------------------------------------------//

    pub struct MockListener {
        addr: SocketAddr,
        empty: Vec<Cursor<Vec<u8>>>,
    }

    impl MockListener {
        fn new(addr: SocketAddr) -> MockListener {
            MockListener {
                addr: addr,
                empty: vec![
                Cursor::new(vec![255;10]),
                Cursor::new(vec![255;10]),
                Cursor::new(vec![255;10]),
                Cursor::new(vec![255;10]),
                Cursor::new(vec![255;10]),
                Cursor::new(vec![255;10]),
                ],
            }
        }
    }

    impl LocalAddr for MockListener {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(self.addr)
        }
    }

    impl Stream for MockListener{
        type Item = Cursor<Vec<u8>>;
        fn poll(&mut self) -> io::Result<Self::Item> {
           if let Some(v) = self.empty.pop(){
               Ok(v)
           }else {
               Err(Error::new(ErrorKind::NotFound, ()))
           }
        }
    }
}
