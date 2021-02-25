
mod manager;
pub use manager::config::HandshakerConfig;
pub use manager::{HandshakerManagerBuilder, HandshakerManagerSink, HandshakerManagerStream};

pub mod handler;

mod filter;
pub use filter::{FilterDecision, HandshakeFilter, HandshakeFilters};

mod message;
pub use message::complete::CompleteMessage;
pub use message::extensions::{Extension, Extensions};
pub use message::initiate::InitiateMessage;
pub use message::protocol::Protocol;

/// Built in objects implementing `Transport`.
pub mod transports {
    pub use super::transport::{TcpListenerStream, TcpTransport};
}

mod transport;
pub use transport::Transport;

mod stream;
pub use stream::Stream;

mod local_addr;
pub use local_addr::LocalAddr;

mod discovery;
pub use discovery::DiscoveryInfo;

pub use crate::util::bt::{InfoHash, PeerId};
