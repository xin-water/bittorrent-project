//! Generic `PeerProtocol` implementations.

use std::io::{self, Write};

use bytes::Bytes;

pub mod codec;

/// Trait for implementing a bittorrent protocol message.
pub trait MessageCodec {
    /// Type of message the protocol operates with.
    type Message;

    /// Total number of bytes needed to parse a complete message. This is not
    /// in addition to what we were given, this is the total number of bytes, so
    /// if the given bytes has length >= needed, then we can parse it.
    ///
    /// If none is returned, it means we need more bytes to determine the number
    /// of bytes needed. If an error is returned, it means the connection should
    /// be dropped, as probably the message exceeded some maximum length.
    fn bytes_needed(&mut self, bytes: &[u8]) -> io::Result<Option<usize>>;

    /// Parse a `ProtocolMessage` from the given bytes.
    fn parse_bytes(&mut self, bytes: Bytes) -> io::Result<Self::Message>;

    /// Write a `ProtocolMessage` to the given writer.
    fn write_bytes<W>(&mut self, message: &Self::Message, writer: W) -> io::Result<()>
    where
        W: Write;

    /// Retrieve how many bytes the message will occupy on the wire.
    fn message_size(&mut self, message: &Self::Message) -> usize;

}
