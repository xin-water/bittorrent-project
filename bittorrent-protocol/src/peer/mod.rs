
#[macro_use]
mod macros;

mod message;

/// Serializable and deserializable protocol messages.
pub mod messages {
    pub use crate::peer::message::{
        BitFieldIter, BitFieldMessage, BitsExtensionMessage, CancelMessage, ExtendedMessage,
        ExtendedType, HaveMessage, NullProtocolMessage, PeerExtensionProtocolMessage,
        PeerWireProtocolMessage, PieceMessage, PortMessage, RequestMessage, UtMetadataDataMessage,
        UtMetadataMessage, UtMetadataRejectMessage, UtMetadataRequestMessage,
    };

    /// Builder types for protocol messages.
    pub mod builders {
        pub use crate::peer::message::ExtendedMessageBuilder;
    }
}

mod message_codec;
pub use message_codec::MessageCodec;
pub use message_codec::codec::PeerWireMessageCodec;
pub use message_codec::tokio_codec::PeerTokioCodec;

mod manager;
pub use manager::{
    IPeerManagerMessage, ManagedMessage, MessageId, OPeerManagerMessage, PeerManager,
    PeerManagerSink, PeerManagerStream,
};
pub use manager::builder::PeerManagerBuilder;
pub use manager::peer_info::PeerInfo;

/// `PeerManager` error types.
pub mod error {
    pub use super::manager::error::{
        PeerManagerError, PeerManagerErrorKind, PeerManagerResult, PeerManagerResultExt,
    };
}
