//! Codecs operating over `PeerProtocol`s.

use std::io;

use crate::peer::MessageCodec;
use bytes::{BufMut, BytesMut};
use tokio_codec::{Decoder, Encoder};

/// Codec operating over some `PeerProtocol`.
pub struct PeerTokioCodec<P> {
    protocolcodec: P,
    max_payload: Option<usize>,
}

impl<P> PeerTokioCodec<P> {
    /// Create a new `PeerProtocolCodec`.
    ///
    /// It is strongly recommended to use `PeerProtocolCodec::with_max_payload`
    /// instead of this function, as this function will not enforce a limit on
    /// received payload length.
    pub fn new(protocolcodec: P) -> PeerTokioCodec<P> {
        PeerTokioCodec {
            protocolcodec: protocolcodec,
            max_payload: None,
        }
    }

    /// Create a new `PeerProtocolCodec` which will yield an error if
    /// receiving a payload larger than the specified `max_payload`.
    pub fn with_max_payload(protocolcodec: P, max_payload: usize) -> PeerTokioCodec<P> {
        PeerTokioCodec {
            protocolcodec: protocolcodec,
            max_payload: Some(max_payload),
        }
    }
}

impl<P> Decoder for PeerTokioCodec<P>
where
    P: MessageCodec,
{
    type Item = P::Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        let src_len = src.len();

        let bytes = match self.protocolcodec.bytes_needed(src.as_ref())? {
            Some(needed)
                if self
                    .max_payload
                    .map(|max_payload| needed > max_payload)
                    .unwrap_or(false) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "PeerProtocolCodec Enforced Maximum Payload Check For Peer",
                ))
            }
            Some(needed) if needed <= src_len => src.split_to(needed).freeze(),
            Some(_) | None => return Ok(None),
        };

        self.protocolcodec
            .parse_bytes(bytes)
            .map(|message| Some(message))
    }
}

impl<P> Encoder for PeerTokioCodec<P>
where
    P: MessageCodec,
{
    type Item = P::Message;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> io::Result<()> {
        dst.reserve(self.protocolcodec.message_size(&item));

        self.protocolcodec.write_bytes(&item, dst.writer())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use crate::peer::{MessageCodec,PeerTokioCodec};
    use bytes::{Bytes, BytesMut};
    use tokio_io::codec::Decoder;
    use crate::peer::message::ExtendedMessage;

    struct ConsumeProtocol;

    impl MessageCodec for ConsumeProtocol {
        type Message = ();

        fn bytes_needed(&mut self, bytes: &[u8]) -> io::Result<Option<usize>> {
            Ok(Some(bytes.len()))
        }

        fn parse_bytes(&mut self, _bytes: Bytes) -> io::Result<Self::Message> {
            Ok(())
        }

        fn write_bytes<W>(&mut self, _message: &Self::Message, _writer: W) -> io::Result<()>
        where
            W: Write,
        {
            Ok(())
        }

        fn message_size(&mut self, _message: &Self::Message) -> usize {
            0
        }

    }

    #[test]
    fn positive_parse_at_max_payload() {
        let mut codec = PeerTokioCodec::with_max_payload(ConsumeProtocol, 100);
        let mut bytes = BytesMut::with_capacity(100);

        bytes.extend_from_slice(&[0u8; 100]);

        assert_eq!(Some(()), codec.decode(&mut bytes).unwrap());
        assert_eq!(bytes.len(), 0);
    }

    #[test]
    fn negative_parse_above_max_payload() {
        let mut codec = PeerTokioCodec::with_max_payload(ConsumeProtocol, 100);
        let mut bytes = BytesMut::with_capacity(200);

        bytes.extend_from_slice(&[0u8; 200]);

        assert!(codec.decode(&mut bytes).is_err());
        assert_eq!(bytes.len(), 200);
    }
}
