use std::net::TcpStream;
use std::io;
use std::io::{Read, Write};
use crate::utp::UtpSocket;

pub trait TryClone{
    type Item: Read + Write ;
    fn try_clone(&self) ->io::Result<Self::Item>;
}

impl TryClone for TcpStream {
    type Item = TcpStream;

    fn try_clone(&self) -> io::Result<Self::Item> {
       TcpStream::try_clone(self)
    }
}

impl TryClone for UtpSocket {
    type Item =  UtpSocket;

    fn try_clone(&self) -> io::Result<Self::Item> {
        UtpSocket::try_clone(self)
    }
}
