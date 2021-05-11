use std::io;

use super::local_addr::LocalAddr;

use std::io::{Read, Write, Error, ErrorKind};
use std::net::{TcpStream, TcpListener};
use std::net::{SocketAddr, Incoming};
use std::option::Option::Some;
use super::stream::Stream;
use crate::utp::{UtpSocket, UtpListener, UtpStream};

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
    type Listener = TcpListenerStream;

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

    fn poll(&mut self) -> io::Result<(TcpStream,SocketAddr)> {
        if let Ok((result,addr)) = self.listener.accept() {
            Ok((result,addr))
        }else {
            Err(Error::new(ErrorKind::NotFound, "listener fail"))
        }
    }
}


/// Defines a `Transport` operating over UTP.
pub struct UtpTransport;


impl Transport for UtpTransport {
    type Socket = UtpSocket;
    type Listener = UtpListenerStream;

    fn connect(&self, addr: &SocketAddr) -> io::Result<Self::Socket> {
        UtpSocket::connect(addr)
    }

    fn listen(&self, addr: &SocketAddr) -> io::Result<Self::Listener> {
        let listener = UtpListener::bind(addr)?;
        let listen_addr = listener.local_addr()?;

        Ok(UtpListenerStream::new(listen_addr, listener))
    }
}


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

    fn poll(&mut self) -> io::Result<(UtpSocket,SocketAddr)> {
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
