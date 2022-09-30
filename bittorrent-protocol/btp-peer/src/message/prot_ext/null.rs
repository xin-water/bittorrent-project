use bytes::Bytes;
use std::io::{self, Write};

/// Enumeration of messages for `NullProtocol`.
#[derive(Debug,PartialEq)]
pub enum NullProtocolMessage {}

impl NullProtocolMessage{

    fn bytes_needed(&self, _bytes: &[u8]) -> io::Result<Option<usize>> {
        Ok(Some(0))
    }

    fn parse_bytes(&self, _bytes: Bytes) -> io::Result<NullProtocolMessage> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Attempted To Parse Bytes As Null Protocol",
        ))
    }

    pub(crate) fn write_bytes<W>(&self, _writer: W) -> io::Result<()>
        where
            W: Write,
    {
        panic!("bittorrent-protocol_peer: NullProtocol::write_bytes Was Called...Wait, How Did You Construct An Instance Of NullProtocolMessage? :)")
    }

    pub(crate) fn message_size(&self) -> usize {
        0
    }

}

