use std::io;
use std::net::{SocketAddr};
use std::sync::Arc;

use tokio::{sync::mpsc, net::UdpSocket, task};

use crate::router::Router;
use crate::routing::table::{self, RoutingTable};
use crate::transaction::TransactionID;
use btp_util::bt::InfoHash;
use crate::DhtBuilder;
use crate::worker::handler::DhtHandler;
use crate::worker::socket::DhtSocket;

pub mod bootstrap;
pub mod handler;
pub mod lookup;
//pub mod messenger;
pub mod refresh;
pub mod socket;
pub mod timer;

/// Task that our DHT will execute immediately.
// #[derive(Clone)]
#[derive(Debug)]
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

//传递对象不好，应该拆分后传成员的，但我太懒了，后面有需要再改。
pub  async fn start_dht(builder: DhtBuilder)->io::Result<mpsc::UnboundedSender<OneshotTask>>{

    let (command_tx, command_rx) = mpsc::unbounded_channel::<OneshotTask>();

    let udp_socket = UdpSocket::bind(&builder.src_addr).await?;
    let dht_socket = Arc::new(DhtSocket::new(udp_socket)?);


    let socket =dht_socket.clone();
    let (message_tx, mut mesage_rx)=mpsc::channel::<(Vec<u8>, SocketAddr)>(10);
    //专门用一个协程发送数据，避免 数据处理中心 阻塞。
    task::spawn(async move {

        // for (buffer,addr) in mesage_rx.recv().await{
        //     socket.send(&buffer,addr).await;
        // }

        // 这跟上面有啥区别，用上面会出现 发送端发送失败。
        // 通道返回的是option,如果发送端都终止了，这里怎么知道？怎么在发送端终止时退出呢？
        loop {
            if let Some((buffer,addr)) = mesage_rx.recv().await{
                socket.send(&buffer,addr).await;
            }
        }

    });


    let table = RoutingTable::new(table::random_node_id());
    let handler = DhtHandler::new(
        table,
        command_rx,
        dht_socket,
        message_tx,
        builder.read_only,
        builder.announce_port
    );
    task::spawn(handler.run());


    let nodes: Vec<SocketAddr> = builder.nodes.into_iter().collect();
    let routers: Vec<Router> = builder.routers.into_iter().collect();
    if command_tx
        .send(OneshotTask::StartBootstrap(routers, nodes))
        .is_err()
    {
        warn!(
                "bittorrent-protocol_dt: MainlineDht failed to send a start bootstrap message..."
            );
    }


    Ok(command_tx)
}