use std::io::{self};
use std::net::SocketAddr;
use tokio::sync::mpsc;

use crate::server::dispatcher::DispatchMessage;
use crate::ServerHandler;

mod dispatcher;
pub mod handler;

/// Tracker server that executes responses asynchronously.
///
/// Server will shutdown on drop.
pub struct TrackerServer {
    send: mpsc::UnboundedSender<DispatchMessage>,
}

impl TrackerServer {
    /// Run a new TrackerServer.
    pub async fn run<H>(bind: SocketAddr, handler: H) -> io::Result<TrackerServer>
    where
        H: ServerHandler + 'static,
    {
        dispatcher::create_dispatcher(bind, handler)
            .await
            .map(|send|
                TrackerServer { send: send }
            )
    }
}

impl Drop for TrackerServer {
    fn drop(&mut self) {
        self.send
            .send(DispatchMessage::Shutdown)
            .expect("bittorrent-protocol_utracker: TrackerServer Failed To Send Shutdown Message");
    }
}
