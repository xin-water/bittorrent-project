
#[macro_use]
extern crate nom;

mod manager;
pub use manager::config::HandshakerConfig;
pub use manager::{HandshakerManagerBuilder, HandshakerManagerSink, HandshakerManagerStream};

pub mod handler;

mod filter;
pub use filter::{FilterDecision, HandshakeFilter, HandshakeFilters};

mod message;
pub use manager::out_msg::CompleteMessage;
pub use message::extensions::{Extension, Extensions};
pub use manager::in_msg::InitiateMessage;
pub use message::protocol::Protocol;

/// Built in objects implementing `Transport`.
pub mod transports {
    pub use super::transport::{TcpListenerStream, TcpTransport,UtpListenerStream, UtpTransport};
}

mod transport;
pub use transport::Transport;

mod stream;
pub use stream::Stream;

mod local_addr;
pub use local_addr::LocalAddr;

mod discovery;
pub use discovery::DiscoveryInfo;

pub use btp_util::bt::{InfoHash, PeerId};
