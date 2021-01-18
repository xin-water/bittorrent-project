use std::io;

use super::local_addr::LocalAddr;

use std::io::{Read, Write, Error, ErrorKind};
use std::net::{TcpStream, TcpListener};
use std::net::{SocketAddr, Incoming};
use std::option::Option::Some;
use super::stream::Stream;

/// Trait for initializing connections over an abstract `Transport`.
pub trait Transport {
    /// Concrete socket.
    type Socket: Read + Write + 'static;

    /// Concrete listener.
    type Listener: Stream<Item = (Self::Socket, SocketAddr) > + LocalAddr + 'static;

    /// Connect to the given address over this transport, using the supplied `Handle`.
    fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket>;

    /// Listen to the given address for this transport, using the supplied `Handle`.
    fn listen(&self, addr: &SocketAddr ) -> io::Result<Self::Listener>;
}

//----------------------------------------------------------------------------------//

/// Defines a `Transport` operating over TCP.
pub struct TcpTransport;

impl Transport for TcpTransport {
    type Socket = TcpStream;
    type Listener = TcpListenerStream<TcpListener>;

    fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket> {
        TcpStream::connect(addr)
    }

    fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
        let listener = TcpListener::bind(addr)?;
        let listen_addr = listener.local_addr()?;

        Ok(TcpListenerStream::new(listen_addr, listener))
    }
}

/// Convenient object that wraps a listener stream `L`, and also implements `LocalAddr`.
pub struct TcpListenerStream<L> {
    listen_addr: SocketAddr,
    listener: L,
}

impl<L> TcpListenerStream<L> {

    fn new(listen_addr: SocketAddr, listener: L) -> TcpListenerStream<L> {
        TcpListenerStream {
            listen_addr: listen_addr,
            listener: listener,
        }
    }
}

impl<L> LocalAddr for TcpListenerStream<L> {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.listen_addr)
    }
}

impl Stream for TcpListenerStream<TcpListener>  {
    type Item = (TcpStream,SocketAddr);

    fn poll(&mut self) -> io::Result<(TcpStream,SocketAddr)> {
        if let Ok((result,addr)) = self.listener.accept() {
            Ok((result,addr))
        }else {
            Err(Error::new(ErrorKind::NotFound, "listener fail"))
        }
    }
}

//----------------------------------------------------------------------------------//

#[cfg(test)]
pub mod test_transports {
    use std::io::{self, Cursor, Error, ErrorKind};
    use std::net::SocketAddr;

    use super::Transport;
    use crate::handshake::LocalAddr;
    use crate::handshake::stream::Stream;


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
