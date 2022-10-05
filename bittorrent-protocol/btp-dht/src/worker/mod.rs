use std::io;
use std::net::{SocketAddr};

use tokio::{
    sync::mpsc,
    net::UdpSocket,
};
use mio;
use tokio::sync::oneshot;

use crate::router::Router;
use crate::routing::table::{self, RoutingTable};
use crate::transaction::TransactionID;
use btp_util::bt::InfoHash;
use crate::worker::socket::Socket;

pub mod bootstrap;
pub mod handler;
pub mod lookup;
//pub mod messenger;
pub mod refresh;
pub mod socket;
pub mod timer;

/// Task that our DHT will execute immediately.
// #[derive(Clone)]
pub enum OneshotTask {
    /// Register a sender to send DhtEvents to.
    RegisterSender(mpsc::UnboundedSender<DhtEvent>),
    /// Load a new bootstrap operation into worker storage.
    StartBootstrap(Vec<Router>, Vec<SocketAddr>),
    /// Start a lookup for the given InfoHash.
    StartLookup(InfoHash, bool ,mpsc::UnboundedSender<SocketAddr>),
    /// Gracefully shutdown the DHT and associated workers.
    Shutdown(ShutdownCause),
}

/// Task that our DHT will execute some time later.
#[derive(Copy, Clone, Debug)]
pub enum ScheduledTask {
    /// Check the progress of the bucket refresh.
    CheckTableRefresh(TransactionID),
    /// Check the progress of the current bootstrap.
    CheckBootstrapTimeout(TransactionID),
    /// Check the progress of a current lookup.
    CheckLookupTimeout(TransactionID),
    /// Check the progress of the lookup endgame.
    CheckLookupEndGame(TransactionID),
}

/// Event that occured within the DHT which clients may be interested in.
#[derive(Copy, Clone, Debug)]
pub enum DhtEvent {
    /// DHT completed the bootstrap.
    BootstrapCompleted,
    /// Lookup operation for the given InfoHash completed.
    LookupCompleted(InfoHash),
    /// DHT is shutting down for some reason.
    ShuttingDown(ShutdownCause),
}

/// Event that occured within the DHT which caused it to shutdown.
#[derive(Copy, Clone, Debug)]
pub enum ShutdownCause {
    /// DHT failed to bootstrap more than once.
    BootstrapFailed,
    /// Client controlling the DHT intentionally shut it down.
    ClientInitiated,
    /// Cause of shutdown is not specified.
    Unspecified,
}