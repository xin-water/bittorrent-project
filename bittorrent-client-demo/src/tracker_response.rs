use std::net::Ipv4Addr;

use bencode;
use bencode::{Bencode, FromBencode};
use bencode::util::ByteString;

use crate::decoder;
use crate::tracker_response::Error::{decoderError, streamingError};

#[derive(Debug, PartialEq)]
pub struct TrackerResponse {
    pub interval: u32,
    pub min_interval: Option<u32>,
    pub complete: Option<u32>,
    pub incomplete: Option<u32>,
    pub peers_u8: Option<Vec<u8>>,
    pub peers: Option<Vec<Peer>>,
    //TrackerResponse状态
    pub is_bad: Option<bool>,
}

impl FromBencode for TrackerResponse {
    type Err = decoder::Error;

    fn from_bencode(bencode: &bencode::Bencode) -> Result<TrackerResponse, decoder::Error> {
        match bencode {
            &Bencode::Dict(ref m) => {
                let mut peers: Vec<Peer> = get_field_as_bytes!(m, "peers").chunks(6).map(Peer::from_bytes).collect();
                let mut is_bad = Some(false);
                let peer = peers.pop().unwrap();
                if peer.ip == Ipv4Addr::new(127, 0, 0, 1) {
                    println!("tracker server response err");
                    peers = Vec::new();
                    is_bad = Some(true);
                } else {
                    peers.push(peer);
                }

                let response = TrackerResponse {
                    interval: get_field!(m, "interval").unwrap(),
                    min_interval: get_optional_field!(m, "min interval"),
                    complete: get_field!(m, "complete"),
                    incomplete: get_field!(m, "incomplete"),
                    peers_u8: None,
                    peers: Some(peers),
                    is_bad,
                };
                Ok(response)
            }
            _ => Err(decoder::Error::NotAByteString)
        }
    }
}


impl TrackerResponse {
    pub fn parse(bytes: &[u8]) -> Result<TrackerResponse, Error> {
        let bencode = {
            match bencode::from_buffer(bytes) {
                Ok(v) => v,
                Err(e) => return Err(Error::selfError),
            }
        };

        match FromBencode::from_bencode(&bencode) {
            Ok(v) => Ok(v),
            _ => return Err(Error::selfError),
        }
    }

    pub fn default() -> TrackerResponse {
        TrackerResponse {
            interval: 1,
            min_interval: None,
            complete: Some(3),
            incomplete: Some(4),
            peers_u8: None,
            peers: Some(vec![]),
            is_bad: Some(false),
        }
    }

    pub fn new(bytes: &[u8]) -> Result<TrackerResponse, decoder::Error> {
        let peer = Peer::from_bytes(bytes);
        let peers: Vec<Peer> = vec![peer];
        Ok(
            TrackerResponse {
                interval: 1,
                min_interval: None,
                complete: Some(3),
                incomplete: Some(4),
                peers_u8: None,
                peers: Some(peers),
                is_bad: Some(false),
            }
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct Peer {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl Peer {
    pub fn defalut() -> Peer {
        let ip = Ipv4Addr::new(127, 0, 0, 1);
        let port = 6881;
        Peer { ip: ip, port: port }
    }

    pub fn from_bytes(v: &[u8]) -> Peer {
        let ip = Ipv4Addr::new(v[0], v[1], v[2], v[3]);
        let port = (v[4] as u16) * 256 + (v[5] as u16);
        Peer { ip: ip, port: port }
    }

    pub fn list_from_bytes(v: &[u8]) -> Vec<Peer> {
        let mut peers: Vec<Peer> = Vec::new();
        let mut n = 0;
        let mut m = n + 6;
        for i in 0..(v.len() / 6) {
            peers.push(Peer::from_bytes(&v[n..m]));
            n = m;
            m = n + 6;
        }
        peers
    }
}

#[derive(Debug)]
pub enum Error {
    decoderError(decoder::Error),
    streamingError(bencode::streaming::Error),
    selfError,
}

impl From<decoder::Error> for Error {
    fn from(err: decoder::Error) -> Self {
        decoderError(err)
    }
}

impl From<bencode::streaming::Error> for Error {
    fn from(err: bencode::streaming::Error) -> Self {
        streamingError(err)
    }
}

