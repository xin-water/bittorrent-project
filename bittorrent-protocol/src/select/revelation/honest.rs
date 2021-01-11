use std::collections::HashMap;
use std::collections::HashSet;
// use std::collections::VecDeque;
use std::collections::hash_map::Entry;
use std::collections::vec_deque::VecDeque;

use bit_set::BitSet;
use bytes::{BufMut, BytesMut};

use crate::handshake::InfoHash;
use crate::metainfo::Metainfo;
use crate::peer::messages::{BitFieldMessage, HaveMessage};
use crate::peer::PeerInfo;

use crate::select::revelation::error::{RevealError, RevealErrorKind};
use crate::select::revelation::IRevealMessage;
use crate::select::revelation::ORevealMessage;
use crate::select::ControlMessage;

/// Revelation module that will honestly report any pieces we have to peers.
pub struct HonestRevealModule {
    torrents: HashMap<InfoHash, PeersInfo>,
    out_queue: VecDeque<ORevealMessage>,
    // Shared bytes container to write bitfield messages to
    out_bytes: BytesMut,
}

struct PeersInfo {
    num_pieces: usize,
    status: BitSet<u8>,
    peers: HashSet<PeerInfo>,
}

impl HonestRevealModule {
    /// Create a new `HonestRevelationModule`.
    pub fn new() -> HonestRevealModule {
        HonestRevealModule {
            torrents: HashMap::new(),
            out_queue: VecDeque::new(),
            out_bytes: BytesMut::new(),
        }
    }

    fn add_torrent(&mut self, metainfo: &Metainfo) -> Option<Result<IRevealMessage, RevealError>> {
        let info_hash = metainfo.info().info_hash();

        match self.torrents.entry(info_hash) {
            Entry::Occupied(_) => Some(Err(RevealError::from_kind(
                RevealErrorKind::InvalidMetainfoExists { hash: info_hash },
            ))),
            Entry::Vacant(vac) => {
                let num_pieces = metainfo.info().pieces().count();

                let mut piece_set = BitSet::default();
                piece_set.reserve_len_exact(num_pieces);

                let peers_info = PeersInfo {
                    num_pieces: num_pieces,
                    status: piece_set,
                    peers: HashSet::new(),
                };
                vac.insert(peers_info);

                None
            }
        }
    }

    fn remove_torrent(
        &mut self,
        metainfo: &Metainfo,
    ) -> Option<Result<IRevealMessage, RevealError>> {
        let info_hash = metainfo.info().info_hash();

        if self.torrents.remove(&info_hash).is_none() {
            Some(Err(RevealError::from_kind(
                RevealErrorKind::InvalidMetainfoNotExists { hash: info_hash },
            )))
        } else {
            None
        }
    }

    fn add_peer(&mut self, peer: PeerInfo) -> Option<Result<IRevealMessage, RevealError>> {
        let info_hash = *peer.hash();

        let out_bytes = &mut self.out_bytes;
        let out_queue = &mut self.out_queue;
        self.torrents
            .get_mut(&info_hash)
            .map(|peers_info| {
                // Add the peer to our list, so we send have messages to them
                peers_info.peers.insert(peer);

                // If our bitfield has any pieces in it, send the bitfield, otherwise, dont send it
                if !peers_info.status.is_empty() {
                    // Get our current bitfield, write it to our shared bytes
                    let bitfield_slice = peers_info.status.get_ref().storage();
                    // Bitfield stores index 0 at bit 7 from the left, we want index 0 to be at bit 0 from the left
                    insert_reversed_bits(out_bytes, bitfield_slice);

                    // Split off what we wrote, send this in the message, will be re-used on drop
                    let bitfield_bytes = out_bytes.split_off(0).freeze();
                    let bitfield = BitFieldMessage::new(bitfield_bytes);

                    // Enqueue the bitfield message so that we send it to the peer
                    out_queue.push_back(ORevealMessage::SendBitField(peer, bitfield));
                }

                None
            })
            .unwrap_or_else(|| {
                Some(Err(RevealError::from_kind(
                    RevealErrorKind::InvalidMetainfoNotExists { hash: info_hash },
                )))
            })
    }

    fn remove_peer(&mut self, peer: PeerInfo) -> Option<Result<IRevealMessage, RevealError>> {
        let info_hash = *peer.hash();

        self.torrents
            .get_mut(&info_hash)
            .map(|peers_info| {
                peers_info.peers.remove(&peer);

                None
            })
            .unwrap_or_else(|| {
                Some(Err(RevealError::from_kind(
                    RevealErrorKind::InvalidMetainfoNotExists { hash: info_hash },
                )))
            })
    }

    fn insert_piece(
        &mut self,
        hash: InfoHash,
        index: u64,
    ) -> Option<Result<IRevealMessage, RevealError>> {
        let out_queue = &mut self.out_queue;
        self.torrents
            .get_mut(&hash)
            .map(|peers_info| {
                if index as usize >= peers_info.num_pieces {
                    Some(Err(RevealError::from_kind(
                        RevealErrorKind::InvalidPieceOutOfRange {
                            index: index,
                            hash: hash,
                        },
                    )))
                } else {
                    // Queue up all have messages
                    for peer in peers_info.peers.iter() {
                        out_queue.push_back(ORevealMessage::SendHave(
                            *peer,
                            HaveMessage::new(index as u32),
                        ));
                    }

                    // Insert into bitfield
                    peers_info.status.insert(index as usize);

                    None
                }
            })
            .unwrap_or_else(|| {
                Some(Err(RevealError::from_kind(
                    RevealErrorKind::InvalidMetainfoNotExists { hash: hash },
                )))
            })
    }
}

/// Inserts the slice into the `BytesMut` but reverses the bits in each byte.
fn insert_reversed_bits(bytes: &mut BytesMut, slice: &[u8]) {
    for mut byte in slice.iter().map(|byte| *byte) {
        let mut reversed_byte = 0;

        for _ in 0..8 {
            // Make room for the bit
            reversed_byte <<= 1;
            // Push the last bit over
            reversed_byte |= byte & 0x01;
            // Push the last bit off
            byte >>= 1;
        }

        bytes.put_u8(reversed_byte);
    }
}

impl HonestRevealModule {
    pub fn send(&mut self, item: IRevealMessage) -> Option<Result<IRevealMessage, RevealError>> {
        let result = match item {
            IRevealMessage::Control(ControlMessage::AddTorrent(metainfo)) => {
                self.add_torrent(&metainfo)
            }
            IRevealMessage::Control(ControlMessage::RemoveTorrent(metainfo)) => {
                self.remove_torrent(&metainfo)
            }
            IRevealMessage::Control(ControlMessage::PeerConnected(info)) => self.add_peer(info),
            IRevealMessage::Control(ControlMessage::PeerDisconnected(info)) => {
                self.remove_peer(info)
            }
            IRevealMessage::FoundGoodPiece(hash, index) => self.insert_piece(hash, index),
            IRevealMessage::Control(ControlMessage::Tick(_))
            | IRevealMessage::ReceivedBitField(_, _)
            | IRevealMessage::ReceivedHave(_, _) => None,
        };

        result
    }
}

impl HonestRevealModule {
   pub fn poll(&mut self) -> Result<Option<ORevealMessage>, RevealError> {
        Ok(self.out_queue.pop_front())
    }
}
