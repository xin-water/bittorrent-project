use std::collections::HashSet;
use std::io;
use std::net::{SocketAddr};
use log::warn;

use tokio::{
    sync::{mpsc,oneshot},
    task,
    net::UdpSocket
};
use tokio::sync::mpsc::Receiver;

use btp_util::bt::InfoHash;
use btp_util::net;

use crate::router::Router;
use crate::routing::table;
use crate::routing::table::RoutingTable;
use crate::worker::{self, DhtEvent, OneshotTask, ShutdownCause, start_dht};
use crate::worker::handler::DhtHandler;
use crate::worker::socket::DhtSocket;

/// Maintains a Distributed Hash (Routing) Table.
pub struct MainlineDht {
    send: mpsc::UnboundedSender<OneshotTask>,
}

impl MainlineDht {
    /// Start the MainlineDht with the given DhtBuilder and Handshaker.
    async fn with_builder(builder: DhtBuilder) -> io::Result<MainlineDht>
    {

        let command_tx = start_dht(builder).await?;

        Ok(MainlineDht { send: command_tx })
    }

    /// Perform a search for the given InfoHash with an optional announce on the closest nodes.
    ///
    ///
    /// Announcing will place your contact information in the DHT so others performing lookups
    /// for the InfoHash will be able to find your contact information and initiate a handshake.
    ///
    /// If the initial bootstrap has not finished, the search will be queued and executed once
    /// the bootstrap has completed.
    pub async fn search(&self, hash: InfoHash, announce: bool) ->Option<mpsc::UnboundedReceiver<SocketAddr>>{
        let (tx,rx)= mpsc::unbounded_channel();
        if self
            .send
            .send(OneshotTask::StartLookup(hash, announce,tx))
            .is_err()
        {
            warn!("bittorrent-protocol_dht: MainlineDht failed to send a start lookup message...");
            return None;
        };
        Some(rx)
    }

    /// An event Receiver which will receive events occuring within the DHT.
    ///
    /// It is important to at least monitor the DHT for shutdown events as any calls
    /// after that event occurs will not be processed but no indication will be given.
    pub fn events(&self) -> mpsc::UnboundedReceiver<DhtEvent> {
        let (send, recv) = mpsc::unbounded_channel();

        if self.send.send(OneshotTask::RegisterSender(send)).is_err() {
            warn!(
                "bittorrent-protocol_dht: MainlineDht failed to send a register sender message..."
            );
            // TODO: Should we push a Shutdown event through the sender here? We would need
            // to know the cause or create a new cause for this specific scenario since the
            // client could have been lazy and wasnt monitoring this until after it shutdown!
        }

        recv
    }
}

impl Drop for MainlineDht {
    fn drop(&mut self) {
        if self
            .send
            .send(OneshotTask::Shutdown(ShutdownCause::ClientInitiated))
            .is_err()
        {
            warn!(
                "bittorrent-protocol_dht: MainlineDht failed to send a shutdown message (may have already been \
                   shutdown)..."
            );
        }
    }
}

// ----------------------------------------------------------------------------//

/// Stores information for initializing a DHT.
#[derive(Clone, Debug)]
pub struct DhtBuilder {
    pub(crate) nodes: HashSet<SocketAddr>,
    pub(crate) routers: HashSet<Router>,
    pub(crate) read_only: bool,
    pub(crate) src_addr: SocketAddr,
    ext_addr: Option<SocketAddr>,
    pub(crate) announce_port: Option<u16>,
}

impl DhtBuilder {
    /// Create a new DhtBuilder.
    ///
    /// This should not be used directly, force the user to supply builder with
    /// some initial bootstrap method.
    pub fn new() -> DhtBuilder {
        DhtBuilder {
            nodes: HashSet::new(),
            routers: HashSet::new(),
            read_only: true,
            src_addr: net::default_route_v4(),
            ext_addr: None,
            announce_port:None,
        }
    }

    /// Creates a DhtBuilder with an initial node for our routing table.
    pub fn with_node(node_addr: SocketAddr) -> DhtBuilder {
        let dht = DhtBuilder::new();

        dht.add_node(node_addr)
    }

    /// Creates a DhtBuilder with an initial router which will let us gather nodes
    /// if our routing table is ever empty.
    ///
    /// Difference between a node and a router is that a router is never put in
    /// our routing table.
    pub fn with_router(router: Router) -> DhtBuilder {
        let dht = DhtBuilder::new();

        dht.add_router(router)
    }

    /// Add nodes which will be distributed within our routing table.
    pub fn add_node(mut self, node_addr: SocketAddr) -> DhtBuilder {
        self.nodes.insert(node_addr);

        self
    }

    /// Add a router which will let us gather nodes if our routing table is ever empty.
    ///
    /// See DhtBuilder::with_router for difference between a router and a node.
    pub fn add_router(mut self, router: Router) -> DhtBuilder {
        self.routers.insert(router);

        self
    }

    /// Set the read only flag when communicating with other nodes. Indicates
    /// that remote nodes should not add us to their routing table.
    ///
    /// Used when we are behind a restrictive NAT and/or we want to decrease
    /// incoming network traffic. Defaults value is true.
    pub fn set_read_only(mut self, read_only: bool) -> DhtBuilder {
        self.read_only = read_only;

        self
    }

    /// Provide the DHT with our external address. If this is not supplied we will
    /// have to deduce this information from remote nodes.
    ///
    /// Purpose of the external address is to generate a NodeId that conforms to
    /// BEP 42 so that nodes can safely store information on our node.
    pub fn set_external_addr(mut self, addr: SocketAddr) -> DhtBuilder {
        self.ext_addr = Some(addr);

        self
    }

    /// Provide the DHT with the source address.
    ///
    /// If this is not supplied we will use the OS default route.
    pub fn set_source_addr(mut self, addr: SocketAddr) -> DhtBuilder {
        self.src_addr = addr;

        self
    }

    pub fn set_announce_port(mut self, port: u16) -> DhtBuilder {
        self.announce_port = Some(port);

        self
    }

    /// Start a mainline DHT with the current configuration.
    pub async fn start_mainline(self) -> io::Result<MainlineDht>
    {
        MainlineDht::with_builder(self).await
    }
}
