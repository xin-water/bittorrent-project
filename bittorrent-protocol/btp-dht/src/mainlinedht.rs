use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::str::FromStr;
use log::warn;

use tokio::{
    sync::{mpsc},
};

use btp_util::bt::InfoHash;
use btp_util::net;

use crate::router::Router;
use crate::worker::{DhtEvent, OneshotTask, ShutdownCause, start_dht};

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
    pub async fn search(&self, hash: InfoHash, announce: bool) ->Option<mpsc::Receiver<SocketAddr>>{
        let (send,recv)= mpsc::channel::<SocketAddr>(200);
        if self
            .send
            .send(OneshotTask::StartLookup(hash, announce,Some(send)))
            .is_err()
        {
            warn!("bittorrent-protocol_dht: MainlineDht failed to send a start lookup message...");
            return None;
        };
        Some(recv)
    }

    /// An event Receiver which will receive events occuring within the DHT.
    ///
    /// It is important to at least monitor the DHT for shutdown events as any calls
    /// after that event occurs will not be processed but no indication will be given.
    pub fn events(&self) -> Option<mpsc::UnboundedReceiver<DhtEvent>> {
        let (send, recv) = mpsc::unbounded_channel();

        if self.send.send(OneshotTask::RegisterSender(send)).is_err() {
            warn!(
                "bittorrent-protocol_dht: MainlineDht failed to send a register sender message..."
            );
            // TODO: Should we push a Shutdown event through the sender here? We would need
            // to know the cause or create a new cause for this specific scenario since the
            // client could have been lazy and wasnt monitoring this until after it shutdown!
            return None;
        }

        Some(recv)
    }

    pub async fn bootstrapped(&self) -> bool{

        let (send, mut recv) = mpsc::unbounded_channel();

        if self.send.send(OneshotTask::GetBootstrapStatus(send)).is_err() {
            warn!(
                "bittorrent-protocol_dht: MainlineDht failed to send a get bootstrapped sender message..."
            );
            return false;
        }

        recv.recv().await.unwrap_or(false)
    }

    pub fn getStatus(&self) -> Option<mpsc::UnboundedReceiver<DhtEvent>> {
        let (send, recv) = mpsc::unbounded_channel();

        if self.send.send(OneshotTask::RegisterSender(send)).is_err() {
            warn!(
                "bittorrent-protocol_dht: MainlineDht failed to send a register sender message..."
            );
            // TODO: Should we push a Shutdown event through the sender here? We would need
            // to know the cause or create a new cause for this specific scenario since the
            // client could have been lazy and wasnt monitoring this until after it shutdown!
            return None;
        }

        Some(recv)
    }

    /// Start the MainlineDht with the given DhtBuilder and Handshaker.
    pub fn builder() -> DhtBuilder {
        DhtBuilder::new()
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
            src_addr: btp_util::net::default_route_v4(),
            ext_addr: None,
            announce_port: Some(6881),
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
        let mut dht = DhtBuilder::new();
        dht.routers.insert(router);
        dht
    }

    /// Add nodes which will be distributed within our routing table.
    pub fn add_node(mut self, node_addr: SocketAddr) -> DhtBuilder {
        self.nodes.insert(node_addr);

        self
    }

    pub fn add_nodes<T>(mut self, nodes: T) -> DhtBuilder
    where T: IntoIterator<Item=SocketAddr>,
    {
        for node_addr in  nodes.into_iter(){
            self.nodes.insert(node_addr);

        }
        self
    }

    pub fn add_nodes_str<'a,T>(mut self, nodes: T) -> DhtBuilder
        where T: IntoIterator<Item=&'a str>,
    {
        for str_addr in  nodes.into_iter(){
            match SocketAddrV4::from_str(str_addr) {
                Ok(node_addr) => {
                    self.nodes.insert(SocketAddr::V4(node_addr));
                }
                Err(_) => log::error!("{:?}解析失败",str_addr),
            }
        }
        self
    }

    pub fn add_defalut_router(mut self,) -> DhtBuilder {
        self.routers.insert(Router::BitTorrent);
        self.routers.insert(Router::uTorrent);
        self.routers.insert(Router::BitComet);
        self.routers.insert(Router::Transmission);
        self
    }

    /// Add a router which will let us gather nodes if our routing table is ever empty.
    ///
    /// See DhtBuilder::with_router for difference between a router and a node.
    pub fn add_router(mut self, router: &str) -> DhtBuilder {
        match  SocketAddrV4::from_str(router){
            Ok(addr)=>{
                self.routers.insert(Router::Custom(SocketAddr::V4(addr)));
            }
            Err(_)=> log::error!("{:?}解析失败",router),
        }

        self
    }

    pub fn add_routers<'a,T>(mut self, routers: T) -> DhtBuilder
    where  T:IntoIterator<Item=&'a str>
    {
        for str_router in routers.into_iter() {
            match SocketAddrV4::from_str(str_router) {
                Ok(addr) => {
                    self.routers.insert(Router::Custom(SocketAddr::V4(addr)));
                }
                Err(_) => log::error!("{:?}解析失败",str_router),
            }
        }

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
    pub fn set_run_port(mut self, port: u16) -> DhtBuilder {
        self.set_source_addr(
            // 使用`127.0.0.1`会无法发送udp数据包，其他语言我记得用回环地址可以
            // SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127,0,0,1),port))
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0,0,0,0),port))

        )
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
