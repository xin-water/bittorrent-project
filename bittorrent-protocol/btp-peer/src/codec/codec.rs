use std::io::{self, Write};

use super::{MessageCodec};
use crate::message::{BitsExtensionMessage, ExtendedMessage, PeerWireProtocolMessage};

use bytes::Bytes;

/// Protocol for peer wire messages.
pub struct PeerWireMessageCodec {
    our_extended_msg: Option<ExtendedMessage>,
    their_extended_msg: Option<ExtendedMessage>,
}

impl PeerWireMessageCodec {
    /// Create a new `PeerWireProtocol` with the given extension protocol.
    ///
    /// Important to note that nested protocol should follow the same message length format
    /// as the peer wire protocol. This means it should expect a 4 byte (`u32`) message
    /// length prefix. Nested protocols will NOT have their `bytes_needed` method called.
    pub fn new( ) -> PeerWireMessageCodec {
        PeerWireMessageCodec {
            our_extended_msg: None,
            their_extended_msg: None,
        }
    }
}

impl MessageCodec for PeerWireMessageCodec
{
    type Message = PeerWireProtocolMessage;

    fn bytes_needed(&mut self, bytes: &[u8]) -> io::Result<Option<usize>> {
        PeerWireProtocolMessage::bytes_needed(bytes)
    }

    fn parse_bytes(&mut self, bytes: Bytes) -> io::Result<Self::Message> {
        match PeerWireProtocolMessage::parse_bytes(bytes, &self.our_extended_msg) {
            Ok(PeerWireProtocolMessage::BitsExtension(BitsExtensionMessage::Extended(msg))) => {
                self.their_extended_msg = Some(msg.clone());

                Ok(PeerWireProtocolMessage::BitsExtension(
                    BitsExtensionMessage::Extended(msg),
                ))
            }
            other => other,
        }
    }

    fn write_bytes<W>(&mut self, message: &Self::Message, writer: W) -> io::Result<()>
    where
        W: Write,
    {
        match (message.write_bytes(writer, &self.their_extended_msg), message) {
            (
                Ok(()),
                &PeerWireProtocolMessage::BitsExtension(BitsExtensionMessage::Extended(ref msg)),
            ) => {
                self.our_extended_msg = Some(msg.clone());

                Ok(())
            }
            (other, _) => other,
        }

    }

    fn message_size(&mut self, message: &Self::Message) -> usize {
        message.message_size( )
    }

}

#[cfg(test)]
mod test{
    use bytes::Bytes;
    use crate::{MessageCodec, PeerWireMessageCodec};
    use crate::messages::PeerWireProtocolMessage;

    #[test]
    fn test_alive_msg(){
      let alive_data = [0_u8;4];
       let msg= PeerWireMessageCodec::new().parse_bytes(Bytes::from(&alive_data[..]));
        assert_eq!(msg.unwrap(),PeerWireProtocolMessage::KeepAlive)
    }

}
