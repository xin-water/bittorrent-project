//! Module for peer discovery.
use std::net::SocketAddr;

use crate::handshake::InfoHash;
use crate::metainfo::Metainfo;
use crate::peer::messages::UtMetadataMessage;
use crate::peer::PeerInfo;
use crate::select::ControlMessage;
use crate::utracker::announce::ClientState;

pub mod error;

mod ut_metadata;

pub use self::ut_metadata::UtMetadataModule;
use crate::select::discovery::error::DiscoveryError;

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
