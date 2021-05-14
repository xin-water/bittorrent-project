use std::io;
use std::net::SocketAddr;

use super::local_addr::LocalAddr;

use futures::future::Future;
use futures::stream::Stream;
use futures::Poll;
use tokio_core::net::{Incoming, TcpListener, TcpStream, TcpStreamNew};
use tokio_core::reactor::Handle;
use tokio_io::{AsyncRead, AsyncWrite};
use crate::utp::{UtpSocket, UtpListener, UtpStream};

/// Trait for initializing connections over an abstract `Transport`.
pub trait Transport {
    /// Concrete socket.
    type Socket: AsyncRead + AsyncWrite + 'static;

    /// Future `Self::Socket`.
    type FutureSocket: Future<Item = Self::Socket, Error = io::Error> + 'static;

    /// Concrete listener.
    type Listener: Stream<Item = (Self::Socket, SocketAddr), Error = io::Error>
        + LocalAddr
        + 'static;

    /// Connect to the given address over this transport, using the supplied `Handle`.
    fn connect(&self, addr: &SocketAddr, handle: &Handle) -> io::Result<Self::FutureSocket>;

    /// Listen to the given address for this transport, using the supplied `Handle`.
    fn listen(&self, addr: &SocketAddr, handle: &Handle) -> io::Result<Self::Listener>;
}

//----------------------------------------------------------------------------------//

/// Defines a `Transport` operating over TCP.
pub struct TcpTransport;

impl Transport for TcpTransport {
    type Socket = TcpStream;
    type FutureSocket = TcpStreamNew;
    type Listener = TcpListenerStream;

    fn connect(&self, addr: &SocketAddr, handle: &Handle) -> io::Result<Self::FutureSocket> {
        Ok(TcpStream::connect(addr, handle))
    }

    fn listen(&self, addr: &SocketAddr, handle: &Handle) -> io::Result<Self::Listener> {
        let listener = TcpListener::bind(addr, handle)?;
        let listen_addr = listener.local_addr()?;

        Ok(TcpListenerStream::new(listen_addr, listener.incoming()))
    }
}

/// Convenient object that wraps a listener stream `L`, and also implements `LocalAddr`.
pub struct TcpListenerStream {
    listen_addr: SocketAddr,
    listener: Incoming,
}

impl TcpListenerStream {
    fn new(listen_addr: SocketAddr, listener: Incoming) -> TcpListenerStream {
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


impl Stream for TcpListenerStream {

    type Item = (TcpStream, SocketAddr);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.listener.poll()
    }
}


/// Defines a `Transport` operating over UTP.
pub struct UtpTransport;


// impl Transport for UtpTransport {
//     type Socket = UtpSocket;
//     type Listener = UtpListenerStream;
//
//     fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket> {
//         UtpSocket::connect(addr)
//     }
//
//     fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
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

// impl Stream for UtpListenerStream  {
//     type Item = (UtpSocket,SocketAddr);
//
//     fn poll(&mut self) -> io::Result<(UtpSocket,SocketAddr)> {
//         if let Ok((result,addr)) = self.listener.accept() {
//             Ok((result,addr))
//         }else {
//             Err(Error::new(ErrorKind::NotFound, "listener fail"))
//         }
//     }
// }

//----------------------------------------------------------------------------------//

#[cfg(test)]
pub mod test_transports {
    use std::io::{self, Cursor};
    use std::net::SocketAddr;

    use super::Transport;
    use crate::handshake::LocalAddr;

    use futures::future::{self, FutureResult};
    use futures::stream::{self, Empty, Stream};
    use futures::Poll;
    use tokio_core::reactor::Handle;

    pub struct MockTransport;

    impl Transport for MockTransport {
        type Socket = Cursor<Vec<u8>>;
        type FutureSocket = FutureResult<Self::Socket, io::Error>;
        type Listener = MockListener;

        fn connect(&self, _addr: &SocketAddr, _handle: &Handle) -> io::Result<Self::FutureSocket> {
            Ok(future::ok(Cursor::new(Vec::new())))
        }

        fn listen(&self, addr: &SocketAddr, _handle: &Handle) -> io::Result<Self::Listener> {
            Ok(MockListener::new(*addr))
        }
    }

    //----------------------------------------------------------------------------------//

    pub struct MockListener {
        addr: SocketAddr,
        empty: Empty<(Cursor<Vec<u8>>, SocketAddr), io::Error>,
    }

    impl MockListener {
        fn new(addr: SocketAddr) -> MockListener {
            MockListener {
                addr: addr,
                empty: stream::empty(),
            }
        }
    }

    impl LocalAddr for MockListener {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(self.addr)
        }
    }

    impl Stream for MockListener {
        type Item = (Cursor<Vec<u8>>, SocketAddr);
        type Error = io::Error;

        fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
            self.empty.poll()
        }
    }
}
