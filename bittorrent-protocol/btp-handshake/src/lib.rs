
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
pub use manager::InHandshake;
pub use message::protocol::Protocol;

/// Built in objects implementing `Transport`.
pub mod transports {
    pub use crate::socket::transport::{TcpListenerStream, TcpTransport, UtpListenerStream, UtpTransport};
}
mod socket;

pub use socket::transport::Transport;

pub use socket::local_addr::LocalAddr;

pub use manager::discovery::DiscoveryInfo;

pub use btp_util::bt::{InfoHash, PeerId};
