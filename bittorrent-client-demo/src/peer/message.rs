use super::util;
use std::fmt;

#[derive(Clone)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(u32, u32, u32),
    Piece(u32, u32, Vec<u8>),
    Cancel(u32, u32, u32),
    Port, // TODO Add params
}

impl Message {
    pub(crate) fn new(id: &u8, body: &[u8]) -> Message {
        match *id {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Message::Have(util::bytes_to_u32(body)),
            5 => Message::Bitfield(body.to_owned()),
            6 => {
                let index = util::bytes_to_u32(&body[0..4]);
                let offset = util::bytes_to_u32(&body[4..8]);
                let length = util::bytes_to_u32(&body[8..12]);
                Message::Request(index, offset, length)
            }
            7 => {
                let index = util::bytes_to_u32(&body[0..4]);
                let offset = util::bytes_to_u32(&body[4..8]);
                let data = body[8..].to_owned();
                Message::Piece(index, offset, data)
            }
            8 => {
                let index = util::bytes_to_u32(&body[0..4]);
                let offset = util::bytes_to_u32(&body[4..8]);
                let length = util::bytes_to_u32(&body[8..12]);
                Message::Cancel(index, offset, length)
            }
            9 => Message::Port,
            _ => panic!("Bad message id: {}", id),
        }
    }

    pub(crate) fn serialize(self) -> Vec<u8> {
        let mut payload = vec![];
        match self {
            Message::KeepAlive => {}
            Message::Choke => payload.push(0),
            Message::Unchoke => payload.push(1),
            Message::Interested => payload.push(2),
            Message::NotInterested => payload.push(3),
            Message::Have(index) => {
                payload.push(4);
                payload.extend(util::u32_to_bytes(index).into_iter());
            }
            Message::Bitfield(bytes) => {
                payload.push(5);
                payload.extend(bytes);
            }
            Message::Request(index, offset, length) => {
                payload.push(6);
                payload.extend(util::u32_to_bytes(index).into_iter());
                payload.extend(util::u32_to_bytes(offset).into_iter());
                payload.extend(util::u32_to_bytes(length).into_iter());
            }
            Message::Piece(index, offset, data) => {
                payload.push(6);
                payload.extend(util::u32_to_bytes(index).into_iter());
                payload.extend(util::u32_to_bytes(offset).into_iter());
                payload.extend(data);
            }
            Message::Cancel(index, offset, length) => {
                payload.push(8);
                payload.extend(util::u32_to_bytes(index).into_iter());
                payload.extend(util::u32_to_bytes(offset).into_iter());
                payload.extend(util::u32_to_bytes(length).into_iter());
            }
            Message::Port => payload.push(9),
        };

        // prepend size
        let mut size = util::u32_to_bytes(payload.len() as u32);
        size.extend(payload);
        size
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Message::KeepAlive => write!(f, "KeepAlive"),
            Message::Choke => write!(f, "Choke"),
            Message::Unchoke => write!(f, "Unchoke"),
            Message::Interested => write!(f, "Interested"),
            Message::NotInterested => write!(f, "NotInterested"),
            Message::Have(ref index) => write!(f, "Have({})", index),
            Message::Bitfield(ref bytes) => write!(f, "Bitfield({:?})", bytes),
            Message::Request(ref index, ref offset, ref length) => {
                write!(f, "Request({}, {}, {})", index, offset, length)
            }
            Message::Piece(ref index, ref offset, ref data) => {
                write!(f, "Piece({}, {}, size={})", index, offset, data.len())
            }
            Message::Cancel(ref index, ref offset, ref length) => {
                write!(f, "Cancel({}, {}, {})", index, offset, length)
            }
            Message::Port => write!(f, "Port"),
        }
    }
}
