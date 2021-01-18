use bytes::buf::BufMut;
use bytes::BytesMut;
use nom::IResult;
use std::io::{self, Cursor, Write, Read};

use crate::handshake::message::bittorrent::message;
use crate::handshake::message::bittorrent::message::HandshakeMessage;
use std::error::Error;

enum HandshakeState {
    Waiting,
    Length(u8),
    Finished,
}

// We can't use the built in frames because they may buffer more
// bytes than we need for a handshake. That is unacceptable for us
// because we are giving a raw socket to the client of this library.
// We don't want to steal any of their bytes during our handshake!
pub struct FramedHandshake<S> {
    sock: S,
    write_buffer: BytesMut,
    read_buffer: Vec<u8>,
    read_pos: usize,
    state: HandshakeState,
}

impl<S> FramedHandshake<S> {
    pub fn new(sock: S) -> FramedHandshake<S> {
        FramedHandshake {
            sock: sock,
            write_buffer: BytesMut::with_capacity(1),
            read_buffer: vec![0],
            read_pos: 0,
            state: HandshakeState::Waiting,
        }
    }

    pub fn into_inner(self) -> S {
        self.sock
    }
}

impl<S> FramedHandshake<S>
    where
        S: Write,
{
    pub(crate) fn send(&mut self, item: HandshakeMessage) -> Result<(),io::Error> {
        self.write_buffer.reserve(item.write_len());
        item.write_bytes(self.write_buffer.by_ref().writer())?;

        loop {
            let write_result = self.sock.write(&self.write_buffer);

            match write_result {
                Ok(0) => {
                    return Err(
                        io::Error::new(io::ErrorKind::WriteZero, "Failed To Write Bytes").into(),
                    )
                }
                Ok(written) => {
                    self.write_buffer.split_to(written);
                }
                _ => return Ok(()),
            }

            if self.write_buffer.is_empty() {
                self.sock.flush();

                return Ok(());
            }
        }
    }
}

impl<S>  FramedHandshake<S>
    where
        S: Read
{
    pub fn poll(&mut self) -> io::Result<Option<HandshakeMessage>> {
        loop {
            match self.state {
                HandshakeState::Waiting => {
                    let read_result = self
                        .sock
                        .read(&mut self.read_buffer[..]);

                    match read_result {
                        Ok(0) => return Ok(None),
                        Ok(1) => {
                            let length = self.read_buffer[0];

                            self.state = HandshakeState::Length(length);

                            self.read_pos = 1;
                            self.read_buffer = vec![0u8; message::write_len_with_protocol_len(length)];
                            self.read_buffer[0] = length;
                        }
                        Ok(read) => panic!("bittorrent-protocol_handshake: Expected To Read Single Byte, Read {:?}", read),
                        _ => return Ok(None),
                    }
                }
                HandshakeState::Length(length) => {
                    let expected_length = message::write_len_with_protocol_len(length);

                    if self.read_pos == expected_length {
                        match HandshakeMessage::from_bytes(&*self.read_buffer) {
                            IResult::Done(_, message) => {
                                self.state = HandshakeState::Finished;

                                return Ok(Some(message));
                            }
                            IResult::Incomplete(_) => panic!("bittorrent-protocol_handshake: HandshakeMessage Failed With Incomplete Bytes"),
                            IResult::Error(_) => {
                                return Err(io::Error::new(io::ErrorKind::InvalidData, "HandshakeMessage Failed To Parse"))
                            }
                        }
                    } else {

                        let read_buffer = &mut self.read_buffer[self.read_pos..];
                        let read_result = self.sock.read(read_buffer);

                        match read_result {
                            Ok(0) => return Ok(None),
                            Ok(read) => {
                                self.read_pos += read;
                            }
                            _ => return Ok(None),
                        }
                    }
                }
                HandshakeState::Finished => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use std::io::{Cursor, Write};

    use crate::handshake::message::bittorrent::framed::FramedHandshake;
    use crate::handshake::message::bittorrent::message::HandshakeMessage;
    use crate::handshake::message::extensions;
    use crate::handshake::{Extensions, Protocol};
    use crate::util::bt;
    use crate::util::bt::{InfoHash, PeerId};

    fn any_peer_id() -> PeerId {
        [22u8; bt::PEER_ID_LEN].into()
    }

    fn any_info_hash() -> InfoHash {
        [55u8; bt::INFO_HASH_LEN].into()
    }

    fn any_extensions() -> Extensions {
        [255u8; extensions::NUM_EXTENSION_BYTES].into()
    }

    #[test]
    fn positive_write_handshake_message() {
        let message = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );

        let write_frame = FramedHandshake::new(Cursor::new(Vec::new()))
            .send(message.clone())
            .unwrap();

        let recv_buffer = write_frame.into_inner().into_inner();

        let mut exp_buffer = Vec::new();
        message.write_bytes(&mut exp_buffer).unwrap();

        assert_eq!(exp_buffer, recv_buffer);
    }

    #[test]
    fn positive_write_multiple_handshake_messages() {
        let message_one = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );
        let message_two = HandshakeMessage::from_parts(
            Protocol::Custom(vec![5, 6, 7]),
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );

        let write_frame = FramedHandshake::new(Cursor::new(Vec::new()))
            .send(message_one.clone())
            .unwrap()
            .send(message_two.clone())
            .unwrap();

        let recv_buffer = write_frame.into_inner().into_inner();

        let mut exp_buffer = Vec::new();
        message_one.write_bytes(&mut exp_buffer).unwrap();
        message_two.write_bytes(&mut exp_buffer).unwrap();

        assert_eq!(exp_buffer, recv_buffer);
    }

    #[test]
    fn positive_read_handshake_message() {
        let exp_message = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );

        let mut buffer = Vec::new();
        exp_message.write_bytes(&mut buffer).unwrap();

        let mut read_iter = FramedHandshake::new(&buffer[..]).wait();
        let recv_message = read_iter.next().unwrap().unwrap();
        assert!(read_iter.next().is_none());

        assert_eq!(exp_message, recv_message);
    }

    #[test]
    fn positive_read_byte_after_handshake() {
        let exp_message = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );

        let mut buffer = Vec::new();
        exp_message.write_bytes(&mut buffer).unwrap();
        // Write some bytes right after the handshake, make sure
        // our framed handshake doesnt read/buffer these (we need
        // to be able to read them afterwards)
        buffer.write_all(&[55]).unwrap();

        let read_frame = FramedHandshake::new(&buffer[..])
            .ok()
            .unwrap()
            .1;
        let buffer_ref = read_frame.into_inner();

        assert_eq!(&[55], buffer_ref);
    }

    #[test]
    fn positive_read_bytes_after_handshake() {
        let exp_message = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            any_info_hash(),
            any_peer_id(),
        );

        let mut buffer = Vec::new();
        exp_message.write_bytes(&mut buffer).unwrap();
        // Write some bytes right after the handshake, make sure
        // our framed handshake doesnt read/buffer these (we need
        // to be able to read them afterwards)
        buffer.write_all(&[55, 54, 21]).unwrap();

        let read_frame = FramedHandshake::new(&buffer[..])
            .ok()
            .unwrap()
            .1;
        let buffer_ref = read_frame.into_inner();

        assert_eq!(&[55, 54, 21], buffer_ref);
    }
}
