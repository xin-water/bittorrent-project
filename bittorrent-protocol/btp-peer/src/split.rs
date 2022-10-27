use tokio::net::TcpStream;
use std::io;
use std::io::{Read, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use btp_utp::UtpSocket;

pub trait Split{
    type read: AsyncRead + AsyncReadExt + Send + Unpin;
    type write: AsyncWrite + AsyncWriteExt + Send + Unpin;
    fn split(self) ->(Self::read,Self::write);
}

impl Split for TcpStream {
    type read  = OwnedReadHalf;
    type write = OwnedWriteHalf;

    fn split(self) -> (Self::read,Self::write){
       self.into_split()
    }
}

// impl Split for UtpSocket {
//     type Item =  UtpSocket;
//
//     fn try_clone(&self) -> io::Result<Self::Item> {
//         UtpSocket::try_clone(self)
//     }
// }
