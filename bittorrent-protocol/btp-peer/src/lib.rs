
#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate log;

#[macro_use]
extern crate nom;

#[macro_use]
extern crate btp_bencode;

#[macro_use]
mod macros;

mod message;

/// Serializable and deserializable protocol messages.
pub mod messages {
    pub use crate::message::{
        BitFieldIter, BitFieldMessage, BitsExtensionMessage, CancelMessage, ExtendedMessage,
        ExtendedType, HaveMessage, NullProtocolMessage, PeerExtensionProtocolMessage,
        PeerWireProtocolMessage, PieceMessage, PortMessage, RequestMessage, UtMetadataDataMessage,
        UtMetadataMessage, UtMetadataRejectMessage, UtMetadataRequestMessage,
    };

    /// Builder types for protocol messages.
    pub mod builders {
        pub use crate::message::ExtendedMessageBuilder;
    }
}

mod codec;
pub use codec::MessageCodec;
pub use codec::codec::PeerWireMessageCodec;

mod manager;
pub use manager::{
    IPeerManagerMessage, ManagedMessage, MessageId, OPeerManagerMessage, PeerManager,
    PeerManagerSink, PeerManagerStream,
};
pub use manager::builder::PeerManagerBuilder;
pub use handler::peer_info::PeerInfo;

mod handler;
mod split;

/// `PeerManager` error types.
pub mod error {
    pub use super::manager::error::{
        PeerManagerError, PeerManagerErrorKind, PeerManagerResult, PeerManagerResultExt,
    };
}
