//! Module for peer ut_metadata.
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
use crate::select::ut_metadata::error::UtMetadataError;

/// Enumeration of ut_metadata messages that can be sent to a ut_metadata module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IUtMetadataMessage {
    /// Control message.
    Control(ControlMessage),
    /// Find peers and download the metainfo for the `InfoHash`.
    DownloadMetainfo(InfoHash),
    /// Received a UtMetadata message.
    ReceivedUtMetadataMessage(PeerInfo, UtMetadataMessage),
}

/// Enumeration of ut_metadata messages that can be received from a ut_metadata module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OUtMetadataMessage {
    /// Send a UtMetadata message.
    SendUtMetadataMessage(PeerInfo, UtMetadataMessage),
    /// We have finished downloading the given `Metainfo`.
    DownloadedMetainfo(Metainfo),
}
