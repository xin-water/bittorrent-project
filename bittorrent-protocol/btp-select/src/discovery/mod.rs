//! Module for peer discovery.
use std::net::SocketAddr;

use btp_handshake::InfoHash;
use btp_metainfo::Metainfo;
use btp_peer::messages::UtMetadataMessage;
use btp_peer::PeerInfo;
use crate::ControlMessage;
use btp_utracker::announce::ClientState;

pub mod error;

mod ut_metadata;

pub use self::ut_metadata::UtMetadataModule;
use crate::discovery::error::DiscoveryError;

/// Enumeration of discovery messages that can be sent to a discovery module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IDiscoveryMessage {
    /// Control message.
    Control(ControlMessage),
    /// Find peers and download the metainfo for the `InfoHash`.
    DownloadMetainfo(InfoHash),
    /// Received a UtMetadata message.
    ReceivedUtMetadataMessage(PeerInfo, UtMetadataMessage),
}

/// Enumeration of discovery messages that can be received from a discovery module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ODiscoveryMessage {
    /// Send a dht announce for the `InfoHash`.
    SendDhtAnnounce(InfoHash),
    /// Send a udp tracker announce for the `InfoHash`.
    SendUdpTrackerAnnounce(InfoHash, SocketAddr, ClientState),
    /// Send a UtMetadata message.
    SendUtMetadataMessage(PeerInfo, UtMetadataMessage),
    /// We have finished downloading the given `Metainfo`.
    DownloadedMetainfo(Metainfo),
}

pub trait Run {
    fn send(&mut self, item: IDiscoveryMessage) -> Result<Option<IDiscoveryMessage>, DiscoveryError>;
    fn poll(&mut self) -> Option<Result<ODiscoveryMessage, DiscoveryError>>;
}
