//! Library for parsing and writing UDP tracker messages.
//!
//! Includes a default implementation of a bittorrent UDP tracker client
//! and a customizable trait based implementation of a bittorrent UDP tracker
//! server.
// Action ids used in both requests and responses.
const CONNECT_ACTION_ID: u32 = 0;
const ANNOUNCE_IPV4_ACTION_ID: u32 = 1;
const SCRAPE_ACTION_ID: u32 = 2;
const ANNOUNCE_IPV6_ACTION_ID: u32 = 4;

pub mod request;
pub mod response;

pub mod announce;
pub mod contact;
pub mod error;
pub mod option;
pub mod scrape;

mod client;
mod server;

pub use client::error::{ClientError, ClientResult};
pub use client::{ClientMetadata, ClientRequest, ClientResponse, ClientToken, TrackerClient,Handshaker};

pub use server::handler::{ServerHandler, ServerResult};
pub use server::TrackerServer;

pub use crate::util::bt::{InfoHash, PeerId};
