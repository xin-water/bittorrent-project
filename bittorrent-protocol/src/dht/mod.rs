//! Implementation of the Bittorrent Mainline Distributed Hash Table.

// Mainline DHT extensions supported on behalf of libtorrent:
// - Always send 'nodes' on a get_peers response even if 'values' is present
// - Unrecognized requests which contain either an 'info_hash' or 'target' arguments are interpreted as 'find_node'
// - Client identification will be present in all outgoing messages in the form of the 'v' key TODO
// const CLIENT_IDENTIFICATION: &'static [u8] = &[b'B', b'I', b'P', 0, 1];
// * IPv6 is currently NOT supported in this implementation

// TODO: The Vuze dht operates over a protocol that is different than the mainline dht.
// It would be possible to create a dht client that can work over both dhts simultaneously,
// this would require essentially a completely separate routing table of course and so it
// might make sense to make this distinction available to the user and allow them to startup
// two dhts using the different protocols on their own.
// const VUZE_DHT: (&'static str, u16) = ("dht.aelitis.com", 6881);

mod handshake;
pub use self::handshake::DhtDiscoveryInfo;
pub use self::handshake::Handshaker;

#[macro_use]
mod bencode;

mod builder;
pub use builder::{DhtBuilder, MainlineDht};

mod error;

pub mod message;

mod router;
pub use router::Router;

mod routing;

mod security;

mod storage;

mod token;

mod transaction;

mod worker;
pub use worker::{DhtEvent, ShutdownCause};

/// Test
pub use crate::util::bt::{InfoHash, PeerId};
