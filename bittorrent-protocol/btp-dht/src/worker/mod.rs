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
    /// get dht bootstrapStatus
    GetBootstrapStatus(mpsc::UnboundedSender<bool>),
    /// get dht node value
    GetDhtValues(mpsc::UnboundedSender<DhtValues>),
    /// Load a new bootstrap operation into worker storage.
    StartBootstrap(Vec<Router>, Vec<SocketAddr>),
    /// Start a lookup for the given InfoHash.
    StartLookup(InfoHash, bool ,Option<mpsc::Sender<SocketAddr>>),
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
    /// Lookup operation for the given InfoHash announceFailed.
    LookupAnFail(InfoHash),
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


#[derive(Copy, Clone, Debug)]
pub struct DhtValues {
    pub dht_address: SocketAddr,
    pub dht_status: DhtStatus,
    pub good_node_count: usize,
    pub questionable_node_count: usize,
    pub bucket_count: usize,
}

#[derive(Eq, PartialEq, Copy, Clone,Debug)]
pub enum DhtStatus{
    Init,
    BootStrapIng,
    Fail,
    Completed,
}

//传递对象不好，应该拆分后传成员的，但我太懒了，后面有需要再改。
pub  async fn start_dht(builder: DhtBuilder)->io::Result<mpsc::UnboundedSender<OneshotTask>>{

    let (command_tx, command_rx) = mpsc::unbounded_channel::<OneshotTask>();

    let udp_socket = UdpSocket::bind(&builder.src_addr).await?;
    let dht_socket = Arc::new(DhtSocket::new(udp_socket)?);

    let table = RoutingTable::new(table::random_node_id());


    // 专门用一个协程发送数据，避免 数据处理中心 阻塞。
    let socket =dht_socket.clone();
    let (message_tx, mut message_rx)=mpsc::channel::<(Vec<u8>, SocketAddr)>(100);
    //专门用一个协程发送数据，避免 数据处理中心 阻塞。
    task::spawn(async move {

        log::info!("消息发送协程已启动");
        while let Some((buffer,addr)) = message_rx.recv().await{
                socket.send(&buffer,addr).await;
        }
        log::warn!("消息发送协程已退出");

    });


    // 提交 “哈希树广度构建任务” 到消息队列，StartBootstrap负责访问引导节点，快速构建节点哈希树，
    // 大致会为前0～20号桶填充节点，后面桶由于子树本身范围变小，深度较深，可能会没有节点
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

    // 提交 “哈希表树深度构建任务” 到消息队列，
    // 它会以自身id为目标节点，递归查询，不断查找离自身更近的节点，构建20～159号桶。
    // 由于drop为同步方法，故只能使用发送时同步的通道，
    // 半异步通道只能缓存一个任务，所以该任务放到handler new方法中，直接放入预处理任务队列。


    // 节点自动刷新保活任务，为简化程序，该任务在handler构建时放入等待任务队列，节点启动后自动执行


    // 处理中心，他会构建一个循环，三个数据入口：
    // 1号入口：外部命令入口，负责处理外部命令
    // 2号入口：socket消息入口，负责处理从网络中读取到的消息
    // 3号入口：定时器，负责处理超时情况
    let handler = DhtHandler::new(
        table,
        command_rx,
        dht_socket,
        message_tx,
        builder.read_only,
        builder.announce_port
    );
    task::spawn(handler.run());



    Ok(command_tx)
}