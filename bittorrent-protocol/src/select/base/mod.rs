use std::time::Duration;

use crate::metainfo::Metainfo;
use crate::peer::PeerInfo;

/// Enumeration of control messages most modules will be interested in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlMessage {
    /// Start tracking the given torrent.
    AddTorrent(Metainfo),
    /// Stop tracking the given torrent.
    RemoveTorrent(Metainfo),
    /// Connected to the given peer.
    ///
    /// This message can be sent multiple times, which
    /// is useful if extended peer information changes.
    PeerConnected(PeerInfo),
    /// Disconnected from the given peer.
    PeerDisconnected(PeerInfo),
    /// A span of time has passed.
    ///
    /// This message is vital for certain modules
    /// to function correctly. Subsequent durations
    /// should not be spread too far apart.
    Tick(Duration),
}
