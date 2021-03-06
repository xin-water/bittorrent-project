use std::net::SocketAddr;

use crate::util::bt::{InfoHash, PeerId};

/// Trait for peer ut_metadata services to forward peer contact information and metadata.
pub trait Handshaker: Send {
    /// Type that metadata will be passed back to the client as.
    type Metadata: Send;

    /// PeerId exposed to peer ut_metadata services.
    fn id(&self) -> PeerId;

    /// Port exposed to peer ut_metadata services.
    ///
    /// It is important that this is the external port that the peer will be sending data
    /// to. This is relevant if the client employs nat traversal via upnp or other means.
    fn port(&self) -> u16;

    /// Connect to the given address with the InfoHash and expecting the PeerId.
    fn connect(&mut self, expected: Option<PeerId>, hash: InfoHash, addr: SocketAddr);

    /// Send the given Metadata back to the client.
    fn metadata(&mut self, data: Self::Metadata);
}

/// Trait for advertisement information that other peers can discover.
pub trait DhtDiscoveryInfo {
    /// Retrieve our public port that we advertise to others.
    fn port(&self) -> u16;

    /// Retrieve our `PeerId` that we advertise to others.
    fn peer_id(&self) -> PeerId;
}

impl<'a, T> DhtDiscoveryInfo for &'a T
where
    T: DhtDiscoveryInfo,
{
    fn port(&self) -> u16 {
        (*self).port()
    }

    fn peer_id(&self) -> PeerId {
        (*self).peer_id()
    }
}
