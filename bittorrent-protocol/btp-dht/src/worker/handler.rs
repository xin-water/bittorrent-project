use std::collections::HashMap;
use std::convert::AsRef;
use std::io;
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;

use log::Level;

use futures::{
    Stream,
    StreamExt,
};
use tokio::{
    select,
    sync::{mpsc, oneshot}
};
use tokio::sync::mpsc::Sender;
use crate::bencode::Bencode;

use btp_util::bt::InfoHash;
use btp_util::convert;
use btp_util::net::IpAddr;
use crate::error::{DhtError, DhtResult};

use crate::message::announce_peer::{AnnouncePeerResponse, ConnectPort};
use crate::message::compact_info::{CompactNodeInfo, CompactValueInfo};
use crate::message::error::{ErrorCode, ErrorMessage};
use crate::message::find_node::FindNodeResponse;
use crate::message::get_peers::{CompactInfoType, GetPeersResponse};
use crate::message::ping::PingResponse;
use crate::message::request::RequestType;
use crate::message::response:: ResponseType;
use crate::message::MessageType;

use crate::router::Router;

use crate::routing::node::Node;
use crate::routing::node::NodeStatus;
use crate::routing::table::BucketContents;
use crate::routing::table::RoutingTable;

use crate::storage::AnnounceStorage;
use crate::token::{Token, TokenStore};
use crate::transaction::{AIDGenerator, ActionID, TransactionID};

use crate::worker::bootstrap::{BootstrapStatus, TableBootstrap};
use crate::worker::lookup::{LookupStatus, TableLookup};
use crate::worker::refresh::{RefreshStatus, TableRefresh};
use crate::worker::{DhtEvent, OneshotTask, ScheduledTask, ShutdownCause};
use crate::worker::socket::{DhtSocket, IpVersion};
use crate::worker::timer::Timer;

// TODO: Update modules to use find_node on the routing table to update the status of a given node.

const MAX_BOOTSTRAP_ATTEMPTS: usize = 3;
const BOOTSTRAP_GOOD_NODE_THRESHOLD: usize = 10;


/// Actions that we can perform on our RoutingTable.
enum TableAction {
    /// Lookup action.
    Lookup(TableLookup),
    /// Refresh action.
    Refresh(TableRefresh),
    /// Bootstrap action.
    ///
    /// Includes number of bootstrap attempts.
    Bootstrap(TableBootstrap, usize),
}

/// Actions that we want to perform on our RoutingTable after bootstrapping finishes.
enum PostBootstrapAction {
    /// Future lookup action.
    Lookup(InfoHash, bool, Option<mpsc::Sender<SocketAddr>>),
    /// Future refresh action.
    Refresh(TableRefresh, TransactionID),
}

#[derive(Eq, PartialEq)]
pub enum DhtStatus{
    Init,
    BootStrapIng,
    Fail,
    Completed,
}

/// Storage for our EventLoop to invoke actions upon.
pub struct DhtHandler {
    status: DhtStatus,
    status_tx: Vec<mpsc::UnboundedSender<bool>>,
    detached: DetachedDhtHandler,
    table_actions: HashMap<ActionID, TableAction>,
    command_rx: mpsc::UnboundedReceiver<OneshotTask>,
    timer: Timer<ScheduledTask>

}

/// Storage separate from the table actions allowing us to hold mutable references
/// to table actions while still being able to pass around the bulky parameters.
struct DetachedDhtHandler {
    read_only: bool,
    message_in: Arc<DhtSocket>,
    message_out: Sender<(Vec<u8>,SocketAddr)>,
    announce_port: Option<u16>,
    token_store: TokenStore,
    aid_generator: AIDGenerator,
    routing_table: RoutingTable,
    active_stores: AnnounceStorage,
    // If future actions is not empty, that means we are still bootstrapping
    // since we will always spin up a table refresh action after bootstrapping.
    future_actions: Vec<PostBootstrapAction>,
    event_notifiers: Vec<mpsc::UnboundedSender<DhtEvent>>,
}

impl DhtHandler
{
    pub(crate) fn new(
        table: RoutingTable,
        command_rx: mpsc::UnboundedReceiver<OneshotTask>,
        message_in: Arc<DhtSocket>,
        message_out: Sender<(Vec<u8>,SocketAddr)>,
        read_only: bool,
        announce_port: Option<u16>,
    ) -> DhtHandler {
        let mut aid_generator = AIDGenerator::new();

        // Insert the refresh task to execute after the bootstrap
        let mut mid_generator = aid_generator.generate();
        let refresh_trans_id = mid_generator.generate();
        let table_refresh = TableRefresh::new(mid_generator);

        // 添加哈希树深度构建任务与节点保活刷新任务到预处理任务队列
        let future_actions = vec![
            PostBootstrapAction::Lookup(table.node_id(),false,None),
            PostBootstrapAction::Refresh(table_refresh, refresh_trans_id, ),
        ];


        let detached = DetachedDhtHandler {
            read_only: read_only,
            announce_port: announce_port,
            message_in: message_in,
            message_out: message_out,
            token_store: TokenStore::new(),
            aid_generator: aid_generator,
            routing_table: table,
            active_stores: AnnounceStorage::new(),
            future_actions: future_actions,
            event_notifiers: Vec::new(),
        };

        let timer = Timer::new();


        DhtHandler {
            status: DhtStatus::Init,
            status_tx: Vec::new(),
            timer: timer,
            detached: detached,
            table_actions: HashMap::new(),
            command_rx: command_rx,
        }
    }

    pub async fn run(mut self){
        while self.status != DhtStatus::Fail {
            self.run_one().await
        }
    }

    pub async fn run_one(&mut self){
        select! {
            command = self.command_rx.recv() => {

                log::trace!("command_rx: {:?}",&command);
                if let Some(command) = command {
                    self.handle_command(command).await
                } else {
                    self.shutdown()
                }
            }
            message = self.detached.message_in.recv() => {
                log::trace!("message_rx: {:?}",&message);

                match message {
                    Ok((buffer, addr)) =>  self.handle_incoming(&buffer, addr).await,
                    Err(error) => log::warn!("{}: Failed to receive incoming message: {}", self.ip_version(), error),
                }
            }
            token = self.timer.next(), if !self.timer.is_empty() => {
                // `unwrap` is OK because we checked the timer is non-empty, so it should never
                // return `None`.
                let token = token.unwrap();
                log::trace!("timeout_rx: {:?}",&token);
                self.handle_timeout(token).await
            }
        }
    }

    fn ip_version(&self) ->&str{
        match self.detached.message_in.ip_version() {
            IpVersion::V4 => "4",
            IpVersion::V6 => "6",
        }
    }

    /// Number of good nodes in the RoutingTable.
    fn num_good_nodes(&self) -> usize {
        let table = &self.detached.routing_table;

        table.closest_nodes(table.node_id())
            .filter(|n| n.status() == NodeStatus::Good)
            .count()
    }

    /// We should rebootstrap if we have a low number of nodes.
    fn nood_to_less(&self) -> bool {
        self.num_good_nodes() <= BOOTSTRAP_GOOD_NODE_THRESHOLD
    }

    /// Broadcast the given event to all of the event nodifiers.
    async fn broadcast_dht_event(&mut self, event: DhtEvent) {
        self.detached.event_notifiers.retain(|send| send.send(event).is_ok());

    }

    fn shutdown(&mut self){
        self.status = DhtStatus::Fail ;
    }

    async fn handle_command(&mut self, command: OneshotTask){
        match command {
            OneshotTask::Shutdown(cause) => {
                self.handle_command_shutdown(cause).await;
            }
            OneshotTask::RegisterSender(send) => {
                self.handle_register_sender(send);
            }
            OneshotTask::StartBootstrap(routers, nodes) => {
                self.handle_start_bootstrap(routers, nodes).await;
            }
            OneshotTask::StartLookup(info_hash, should_announce,tx) => {
                self.handle_start_lookup(info_hash, should_announce, tx).await;
            }
            OneshotTask::GetBootstrapStatus(tx) => {
                self.handle_get_bootstarpped(tx).await;
            }
        }
    }

    async fn handle_incoming(&mut self, buffer: &[u8], addr: SocketAddr){

        // 消息编码
        // 不能抽为函数、会触发返回借用，message包含了 &bencode，
        // 不能抽为方法、会与下面发生可变借用冲突，借用检查算法会误杀。，
        let bencode = if let Ok(b) = Bencode::decode(buffer){
            b
        } else {
            warn!("bittorrent-protocol_dht: Received invalid bencode data...");
            return;
        };
        let message = MessageType::new(&bencode);

        // 预处理
        if !self.handle_incoming_preprocess(&message){
            return;
        }

        match message{
            Err(e) => {
                 warn!("bittorrent-protocol_dht: Error parsing KRPC message: {:?}",e);
             }
             Ok(MessageType::Error(e)) => {
                 warn!("bittorrent-protocol_dht: KRPC error message from {:?}: {:?}",addr, e);
             }
             Ok(MessageType::Request(request)) => {
                 self.handle_incoming_request(request, addr).await
             }
             Ok(MessageType::Response(response)) => {
                 self.handle_incoming_response(response, addr).await
             }
        }

    }

    async fn handle_timeout(&mut self, token: ScheduledTask){
        match token {
            ScheduledTask::CheckTableRefresh(trans_id) => {
                self.handle_check_table_refresh(trans_id).await;
            }
            ScheduledTask::CheckBootstrapTimeout(trans_id) => {
                self.handle_check_bootstrap_timeout(trans_id).await;
            }
            ScheduledTask::CheckLookupTimeout(trans_id) => {
                self.handle_check_lookup_timeout(trans_id).await;
            }
            ScheduledTask::CheckLookupEndGame(trans_id) => {
                self.handle_check_lookup_endgame(trans_id).await;
            }
        }
    }



// ----------------------------------------------------------------------------//


    async fn handle_command_shutdown(&mut self, cause: ShutdownCause)
    {
        self.status_tx.retain(|tx|{
            tx.send(false).expect("dht strapped message send fail ");
            false
        });
        self.broadcast_dht_event(DhtEvent::ShuttingDown(cause)).await;

        self.shutdown();
    }


    async fn handle_get_bootstarpped(&mut self,tx: mpsc::UnboundedSender<bool>) {

        match self.status {
            DhtStatus::Fail=>{tx.send(false).expect("bootstarpped status send fail")}
            DhtStatus::Completed=>{tx.send(true).expect("bootstarpped status send fail")}
            _ => {self.status_tx.push(tx)}
        }
    }

    fn handle_register_sender(&mut self, sender: mpsc::UnboundedSender<DhtEvent>) {
        self.detached.event_notifiers.push(sender);
    }


    async fn handle_start_bootstrap(
        &mut self,
        routers: Vec<Router>,
        nodes: Vec<SocketAddr>,
    )
    {
        let work_storage = &mut self.detached;
        let timer =   &mut self.timer;

        let router_iter = routers
            .into_iter()
            .filter_map(|r| r.ipv4_addr().ok().map(|v4| SocketAddr::V4(v4)));

        let mid_generator = work_storage.aid_generator.generate();
        let action_id = mid_generator.action_id();
        let mut table_bootstrap = TableBootstrap::new(
            work_storage.routing_table.node_id(),
            mid_generator,
            nodes,
            router_iter,
        );

        //广度构建任务可执行多次，所以在原来已经是正常状态时不应该修改状态
        if self.status !=DhtStatus::Completed {
            self.status = DhtStatus::BootStrapIng;
        }

        // Begin the bootstrap operation
        let bootstrap_status = table_bootstrap.start_bootstrap(&work_storage.message_out, timer).await;


        // 启动的时候 只会向初始节点发起请求，所以不可能出现完成事件。
        match bootstrap_status {
            BootstrapStatus::Failed => {
                self.handle_command_shutdown(ShutdownCause::Unspecified).await;
            }
            _ => {
                self.table_actions.insert(action_id, TableAction::Bootstrap(table_bootstrap, 0));
            }
        };

        // if bootstrap_complete {
        //     self.broadcast_bootstrap_completed(action_id).await;
        // }
    }

    /// Attempt to rebootstrap or shutdown the dht if we have no nodes after rebootstrapping multiple time.
    /// Returns None if the DHT is shutting down, Some(true) if the rebootstrap process started, Some(false) if a rebootstrap is not necessary.
    async fn attempt_rebootstrap(
        &mut self,
        mut bootstrap: TableBootstrap,
        mut attempts: usize,
        trans_id: &TransactionID,
    ) -> Option<bool>

    {
        // Increment the bootstrap counter
        attempts += 1;

        warn!(
        "bittorrent-protocol_dht: Bootstrap attempt {} failed, attempting a rebootstrap...",
        attempts
    );

        // Check if we reached the maximum bootstrap attempts
        // todo 不应该关闭 dht ，因为作为初始节点是可以没有其他节点存在的。
        if attempts >= MAX_BOOTSTRAP_ATTEMPTS {
            if self.num_good_nodes() == 0 {
                // Failed to get any nodes in the rebootstrap attempts, shut down
                self.handle_command_shutdown(ShutdownCause::BootstrapFailed).await;
                //启动失败
                None
            } else {
                //不需要重启
                Some(true)
            }
        } else {


            let bootstrap_status = bootstrap.start_bootstrap(&self.detached.message_out, &mut self.timer).await;

            self.table_actions.insert(trans_id.action_id(), TableAction::Bootstrap(bootstrap, attempts));

            match bootstrap_status {
                BootstrapStatus::Failed => {
                    self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                    None
                }
                _ => {
                    //需要重启
                    Some(false)
                }
            }
        }
    }

    /// Broadcast that the bootstrap has completed.
    /// IMPORTANT: Should call this instead of broadcast_dht_event()!
    async fn broadcast_bootstrap_completed(
        &mut self,
        action_id: ActionID
    )
    {
        // Indicates we are out of the bootstrapping phase
        self.status = DhtStatus::Completed;

        self.status_tx.retain(|tx|{
            tx.send(true).expect("dht strapped message send fail ");
            false
        });

        // Send notification that the bootstrap has completed.
        self.broadcast_dht_event(
            DhtEvent::BootstrapCompleted,
        ).await;


        // Remove the bootstrap action from our table actions
        warn!("bootstrap_completed remove bootstrap action_id:{:?}",&action_id);
        self.table_actions.remove(&action_id);


        // Start the post bootstrap actions.
        let mut future_actions = self.detached.future_actions.split_off(0);
        for table_action in future_actions.drain(..) {
            match table_action {
                PostBootstrapAction::Lookup(info_hash, should_announce,tx) => {
                    log::trace!("start future_actions  Lookup info_hash:{:?}",&info_hash);
                    self.handle_start_lookup(
                        info_hash,
                        should_announce,
                        tx
                    ).await;
                }
                PostBootstrapAction::Refresh(refresh, trans_id) => {
                    self.table_actions.insert(trans_id.action_id(), TableAction::Refresh(refresh));
                    log::trace!("start future_actions  Refresh action_id:{:?},trans_id:{:?}",&trans_id.action_id(),&trans_id);
                    self.handle_check_table_refresh(trans_id).await;
                }
            }
        }
    }

    async fn handle_start_lookup(
        &mut self,
        info_hash: InfoHash,
        should_announce: bool,
        tx: Option<mpsc::Sender<SocketAddr>>,
    )
    {
        let mid_generator = self.detached.aid_generator.generate();
        let action_id = mid_generator.action_id();

        if self.status != DhtStatus::Completed {
            // Queue it up if we are currently bootstrapping
            self.detached
                .future_actions
                .push(PostBootstrapAction::Lookup(info_hash, should_announce,tx));
        } else {
            // Start the lookup right now if not bootstrapping
            match TableLookup::new(
                self.detached.routing_table.node_id(),
                info_hash,
                mid_generator,
                should_announce,
                &mut self.detached.routing_table,
                &self.detached.message_out,
                tx,
                &mut self.timer,
            ).await {

                // 这里有2种状态：启动失败与第一次节点为0都算失败，只有搜索中才算成功
                Some(lookup) => {
                    self.table_actions.insert(action_id, TableAction::Lookup(lookup));
                }
                None => self.handle_command_shutdown(ShutdownCause::Unspecified).await,
            }
        }
    }

// ----------------------------------------------------------------------------//

    fn handle_incoming_preprocess(&mut self, message: &DhtResult<MessageType>) -> bool {

        if self.detached.read_only {
            match message {
                Ok(MessageType::Request(_)) =>  return false,
                _ => (),
            }
        }
        true
    }

    async fn handle_incoming_request(
        &mut self,
        request: RequestType<'_>,
        addr: SocketAddr,
    )
    {
        let  work_storage = &mut self.detached;
        match request {
            RequestType::Ping(p) => {
                info!("bittorrent-protocol_dht: Received a PingRequest...");
                let node = Node::as_good(p.node_id(), addr);

                // Node requested from us, mark it in the Routingtable
                work_storage
                    .routing_table
                    .find_node_mut(&node.handle())
                    .map(|n| n.remote_request());

                let ping_rsp =
                    PingResponse::new(p.transaction_id(), work_storage.routing_table.node_id());
                let ping_msg = ping_rsp.encode();

                if work_storage.message_out.send((ping_msg, addr)).await.is_err() {
                    error!(
                        "bittorrent-protocol_dht: Failed to send a ping response on the out channel..."
                    );
                    self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                }
            }
            RequestType::FindNode(f) => {
                info!("bittorrent-protocol_dht: Received a FindNodeRequest...");
                let node = Node::as_good(f.node_id(), addr);

                // Node requested from us, mark it in the Routingtable
                work_storage
                    .routing_table
                    .find_node_mut(&node.handle())
                    .map(|n| n.remote_request());

                // Grab the closest nodes
                let mut closest_nodes_bytes = Vec::with_capacity(26 * 8);
                for node in work_storage
                    .routing_table
                    .closest_nodes(f.target_id())
                    .take(8)
                {
                    closest_nodes_bytes.extend_from_slice(&node.encode());
                }

                let find_node_rsp = FindNodeResponse::new(
                    f.transaction_id(),
                    work_storage.routing_table.node_id(),
                    &closest_nodes_bytes,
                )
                .unwrap();
                let find_node_msg = find_node_rsp.encode();

                if work_storage
                    // .out_channel
                    // .send((find_node_msg, addr))
                    .message_out.send((find_node_msg, addr))
                    .await
                    .is_err()
                {
                    error!("bittorrent-protocol_dht: Failed to send a find node response on the out channel...");
                    self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                }
            }
            RequestType::GetPeers(g) => {
                info!("bittorrent-protocol_dht: Received a GetPeersRequest...");
                let node = Node::as_good(g.node_id(), addr);

                // Node requested from us, mark it in the Routingtable
                work_storage
                    .routing_table
                    .find_node_mut(&node.handle())
                    .map(|n| n.remote_request());

                // TODO: Move socket address serialization code into use btp_util
                // TODO: Check what the maximum number of values we can give without overflowing a udp packet
                // Also, if we arent going to give all of the contacts, we may want to shuffle which ones we give
                let mut contact_info_bytes = Vec::with_capacity(6 * 20);
                work_storage
                    .active_stores
                    .find_items(&g.info_hash(), |addr| {
                        let mut bytes = [0u8; 6];
                        let port = addr.port();

                        match addr {
                            SocketAddr::V4(v4_addr) => {
                                for (src, dst) in convert::ipv4_to_bytes_be(*v4_addr.ip())
                                    .iter()
                                    .zip(bytes.iter_mut())
                                {
                                    *dst = *src;
                                }
                            }
                            SocketAddr::V6(_) => {
                                error!("AnnounceStorage contained an IPv6 Address...");
                                return;
                            }
                        };

                        bytes[4] = (port >> 8) as u8;
                        bytes[5] = (port & 0x00FF) as u8;

                        contact_info_bytes.extend_from_slice(&bytes);
                    });
                // Grab the bencoded list (ugh, we really have to do this, better apis I say!!!)
                let mut contact_info_bencode = Vec::with_capacity(contact_info_bytes.len() / 6);
                for chunk_index in 0..(contact_info_bytes.len() / 6) {
                    let (start, end) = (chunk_index * 6, chunk_index * 6 + 6);

                    contact_info_bencode.push(dht_ben_bytes!(&contact_info_bytes[start..end]));
                }

                // Grab the closest nodes
                let mut closest_nodes_bytes = Vec::with_capacity(26 * 8);
                for node in work_storage
                    .routing_table
                    .closest_nodes(g.info_hash())
                    .take(8)
                {
                    closest_nodes_bytes.extend_from_slice(&node.encode());
                }

                // Wrap up the nodes/values we are going to be giving them
                let token = work_storage
                    .token_store
                    .checkout(IpAddr::from_socket_addr(addr));
                let comapct_info_type = if !contact_info_bencode.is_empty() {
                    CompactInfoType::Both(
                        CompactNodeInfo::new(&closest_nodes_bytes).unwrap(),
                        CompactValueInfo::new(&contact_info_bencode).unwrap(),
                    )
                } else {
                    CompactInfoType::Nodes(CompactNodeInfo::new(&closest_nodes_bytes).unwrap())
                };

                let get_peers_rsp = GetPeersResponse::new(
                    g.transaction_id(),
                    work_storage.routing_table.node_id(),
                    Some(token.as_ref()),
                    comapct_info_type,
                );
                let get_peers_msg = get_peers_rsp.encode();

                if work_storage
                    // .out_channel
                    // .send((get_peers_msg, addr))
                    .message_out.send((get_peers_msg, addr))
                    .await
                    .is_err()
                {
                    error!("bittorrent-protocol_dht: Failed to send a get peers response on the out channel...");
                    self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                }
            }
            RequestType::AnnouncePeer(a) => {
                info!("bittorrent-protocol_dht: Received an AnnouncePeerRequest...");
                let node = Node::as_good(a.node_id(), addr);

                // Node requested from us, mark it in the Routingtable
                work_storage
                    .routing_table
                    .find_node_mut(&node.handle())
                    .map(|n| n.remote_request());

                // Validate the token
                let is_valid = match Token::new(a.token()) {
                    Ok(t) => work_storage
                        .token_store
                        .checkin(IpAddr::from_socket_addr(addr), t),
                    Err(_) => false,
                };

                // Create a socket address based on the implied/explicit port number
                let connect_addr = match a.connect_port() {
                    ConnectPort::Implied => addr,
                    ConnectPort::Explicit(port) => match addr {
                        SocketAddr::V4(v4_addr) => {
                            SocketAddr::V4(SocketAddrV4::new(*v4_addr.ip(), port))
                        }
                        SocketAddr::V6(v6_addr) => SocketAddr::V6(SocketAddrV6::new(
                            *v6_addr.ip(),
                            port,
                            v6_addr.flowinfo(),
                            v6_addr.scope_id(),
                        )),
                    },
                };

                // Resolve type of response we are going to send
                let response_msg = if !is_valid {
                    // Node gave us an invalid token
                    warn!("bittorrent-protocol_dht: Remote node sent us an invalid token for an AnnounceRequest...");
                    ErrorMessage::new(
                        a.transaction_id().to_vec(),
                        ErrorCode::ProtocolError,
                        "Received An Invalid Token".to_owned(),
                    )
                    .encode()
                } else if work_storage
                    .active_stores
                    .add_item(a.info_hash(), connect_addr)
                {
                    // Node successfully stored the value with us, send an announce response
                    AnnouncePeerResponse::new(a.transaction_id(), work_storage.routing_table.node_id())
                        .encode()
                } else {
                    // Node unsuccessfully stored the value with us, send them an error message
                    // TODO: Spec doesnt actually say what error message to send, or even if we should send one...
                    warn!(
                        "bittorrent-protocol_dht: AnnounceStorage failed to store contact information because it \
                           is full..."
                    );
                    ErrorMessage::new(
                        a.transaction_id().to_vec(),
                        ErrorCode::ServerError,
                        "Announce Storage Is Full".to_owned(),
                    )
                    .encode()
                };

                if work_storage.message_out.send((response_msg, addr)).await.is_err() {
                    error!("bittorrent-protocol_dht: Failed to send an announce peer response on the out channel...");
                    self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                }
            }
        }
    }


    async fn handle_incoming_response(
        &mut self,
        response: ResponseType<'_>,
        addr: SocketAddr,
    )
    {
        let  work_storage = &mut self.detached;
        let  table_actions = &mut self.table_actions;
        let  timer = &mut self.timer;

        match response {
            ResponseType::FindNode(f) => {
                info!("bittorrent-protocol_dht: Received a FindNodeResponse...");
                //todo 直接unwrap是不好的行为，此处因为要检查消息序列化正确情况，所以暂时保留。
                let trans_id = TransactionID::from_bytes(f.transaction_id()).unwrap();
                let node = Node::as_good(f.node_id(), addr);

                // todo 要支持ipv6时，这里应该判断本机地址类型，然后再获取对于类型的地址节点
                // Add the payload nodes as questionable
                for (id, v4_addr) in f.nodes() {
                    let sock_addr = SocketAddr::V4(v4_addr);

                    work_storage
                        .routing_table
                        .add_node(Node::as_questionable(id, sock_addr));
                }

                // match返回处理的编程风格
                let bootstrap_complete = {
                    let opt_bootstrap = match table_actions.get_mut(&trans_id.action_id()) {
                        Some(&mut TableAction::Refresh(_)) => {
                            work_storage.routing_table.add_node(node);
                            None
                        }
                        Some(&mut TableAction::Bootstrap(ref mut bootstrap, _attempts)) => {
                            //不是路由节点就加入表中
                            if !bootstrap.is_router(&node.addr()) {
                                work_storage.routing_table.add_node(node);
                            }
                            Some(bootstrap)
                        }

                        _ => {
                            error!(
                            "bittorrent-protocol_dht: Resolved a TransactionID to a FindNodeResponse no action..."
                        );
                            None
                        }
                    };

                    if let Some(bootstrap) = opt_bootstrap {
                        match bootstrap.recv_response(
                            &trans_id,
                            &mut work_storage.routing_table,
                            &work_storage.message_out,
                            timer,
                        ).await {
                            BootstrapStatus::Bootstrapping => false,
                            BootstrapStatus::Failed => {
                                self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                                false
                            }
                            BootstrapStatus::Completed => {

                                // 直接使用 bootstrap引用 会违反可变引用唯一原则
                                // self.attempt_rebootstrap(bootstrap, attempts).await
                                //     == Some(true)

                                if self.nood_to_less() {
                                    if let Some(TableAction::Bootstrap(bootstrap,attempts)) = self.table_actions.remove(&trans_id.action_id()){
                                        self.attempt_rebootstrap(bootstrap, attempts,&trans_id).await == Some(true)
                                    }else {
                                        false
                                    }

                                } else {
                                    true
                                }
                            }
                        }
                    } else {
                        false
                    }
                };

                if bootstrap_complete {
                    self.broadcast_bootstrap_completed(
                        trans_id.action_id()
                    ).await;
                }

                if log_enabled!(Level::Info) {
                    let mut total = 0;

                    for (index, bucket) in self.detached.routing_table.buckets().enumerate() {
                        let num_nodes = match bucket {
                            BucketContents::Empty => 0,
                            BucketContents::Sorted(b) => {
                                b.iter().filter(|n| n.status() == NodeStatus::Good).count()
                            }
                            BucketContents::Assorted(b) => {
                                b.iter().filter(|n| n.status() == NodeStatus::Good).count()
                            }
                        };
                        total += num_nodes;

                        if num_nodes != 0 {
                            print!("Bucket {}: {} | ", index, num_nodes);
                        }
                    }

                    print!("\nTotal: {}\n\n\n", total);
                }
            }
            ResponseType::GetPeers(g) => {
                info!("bittorrent-protocol_dht: Received a GetPeersResponse...");
                //todo 直接unwrap是不好的行为，此处因为要检查消息序列化正确情况，所以暂时保留。
                let trans_id = TransactionID::from_bytes(g.transaction_id()).unwrap();
                //此处获取响应节点，但某些情况消息体内也包含了ip与端口，暂时以udp socket解析为主
                let node = Node::as_good(g.node_id(), addr);

                work_storage.routing_table.add_node(node.clone());

                let lookup = {
                    match table_actions.get_mut(&trans_id.action_id()) {
                        Some(&mut TableAction::Lookup(ref mut lookup)) => lookup,
                        _ => {
                            error!(
                            "bittorrent-protocol_dht: Resolved a TransactionID to a GetPeersResponse but no \
                                action found..."
                        );
                            //其实此处可以直接返回了；
                            return;
                        }
                    }
                };

                    match (lookup.recv_response(
                                node,
                                &trans_id,
                                g,
                                &mut work_storage.routing_table,
                                &work_storage.message_out,
                                timer,).await,
                           lookup.info_hash()  // 为什么必须要在元组里求hash,而不是到事件发布的时候求呢？
                    )                          // 因为 lookup所在引用 与 broadcast_dht_event方法的引用冲突，违反单一可变。
                    {

                        // 此处只会有3种状态，消息发送失败、不需要最后循环的成功、以及正常继续搜索
                        (LookupStatus::Completed,infohash) => self.broadcast_dht_event(
                            DhtEvent::LookupCompleted(infohash)
                        ).await,
                        (LookupStatus::Failed,_) => {
                            self.handle_command_shutdown(ShutdownCause::Unspecified).await
                        }
                        _ => (),
                    }

            }
            ResponseType::Ping(_) => {
                info!("bittorrent-protocol_dht: Received a PingResponse...");

                // Yeah...we should never be getting this type of response (we never use this message)
            }
            ResponseType::AnnouncePeer(_) => {
                info!("bittorrent-protocol_dht: Received an AnnouncePeerResponse...");
            }
            ResponseType::Acting(a) => {
                info!("bittorrent-protocol_dht: Received a ActingResponse...");
                let node = Node::as_good(a.node_id(), addr);

                work_storage.routing_table.add_node(node);

                //todo,可能需要根据事物id进一步判断具体类型，做一些可能的事后处理，不是很重要，暂不处理。
            }
        }
    }







async fn handle_check_table_refresh(
    &mut self,
    trans_id: TransactionID,
)
{
    let opt_refresh_status = match self.table_actions.get_mut(&trans_id.action_id()) {
        Some(&mut TableAction::Refresh(ref mut refresh)) => Some(refresh.continue_refresh(
            &mut self.detached.routing_table,
            &self.detached.message_out,
            &mut self.timer,
        ).await),
        _ => {
            error!(
                "bittorrent-protocol_dht: Resolved a TransactionID to a check table refresh but no action \
                    found..."
            );
            None
        }
    };

    match opt_refresh_status {
        None => (),
        Some(RefreshStatus::Refreshing) => (),
        Some(RefreshStatus::Failed) => self.handle_command_shutdown(ShutdownCause::Unspecified).await,
    }
}

async fn handle_check_bootstrap_timeout(
    &mut self,
    trans_id: TransactionID,
)
{

    let bootstrap_complete = {
        let opt_bootstrap_info = match self.table_actions.get_mut(&trans_id.action_id()) {
            Some(&mut TableAction::Bootstrap(ref mut bootstrap,  _attempts)) => Some(
                bootstrap.recv_timeout(
                    &trans_id,
                    &mut self.detached.routing_table,
                    &self.detached.message_out,
                    &mut self.timer,
                ).await),
            _ => {
                error!(
                    "bittorrent-protocol_dht: Resolved a TransactionID to a check table bootstrap..."
                );
                None
            }
        };

        match opt_bootstrap_info {
            Some(BootstrapStatus::Failed) => {
                self.handle_command_shutdown(ShutdownCause::Unspecified).await;
                false
            }
            Some(BootstrapStatus::Completed) => {
                if self.nood_to_less() {
                    //直接使用 上面的bootstrap引用 会违反可变引用唯一原则
                    // self.attempt_rebootstrap(bootstrap, attempts).await
                    //     == Some(true)
                    if let Some(TableAction::Bootstrap(bootstrap,attempts)) = self.table_actions.remove(&trans_id.action_id()){
                        self.attempt_rebootstrap(bootstrap, attempts,&trans_id).await == Some(true)
                    }else {
                        false
                    }
                }else {
                    true
                }
            }
            _=> false,
        }
    };

    if bootstrap_complete {
        self.broadcast_bootstrap_completed(
            trans_id.action_id()
        ).await;
    }
}

async fn handle_check_lookup_timeout(
    &mut self,
    trans_id: TransactionID,
)
{
    let (work_storage, table_actions) = (&mut self.detached, &mut self.table_actions);

    let opt_lookup_info = match table_actions.get_mut(&trans_id.action_id()) {
        Some(&mut TableAction::Lookup(ref mut lookup)) => (
            lookup.recv_timeout(
                &trans_id,
                &mut work_storage.routing_table,
                &work_storage.message_out,
                &mut self.timer,
            ).await,
            lookup.info_hash(),
        ),
        _ => {
            error!(
                "bittorrent-protocol_dht: Resolved a TransactionID to a check table lookup..."
            );
            //其他情况直接返回，没必要给None,后面在一起判断。
            return;
        }
    };

    // 这里有3种状态：搜索中、不需要启动最后循环时的成功、启动最后循环失败
    // 后面两种状态需要处理
    match opt_lookup_info {
        (LookupStatus::Completed, info_hash) => self.broadcast_dht_event(
            DhtEvent::LookupCompleted(info_hash),
        ).await,
        (LookupStatus::Failed, _) => {
            self.handle_command_shutdown(ShutdownCause::Unspecified).await
        }
        _ => (),
    }
}

async fn handle_check_lookup_endgame(
    &mut self,
    trans_id: TransactionID,
)
{
    let (work_storage, table_actions) = (&mut self.detached, &mut self.table_actions);

    warn!("remove lookup action_id:{:?}",&trans_id.action_id());

    let lookup_info = match table_actions.remove(&trans_id.action_id()) {
        Some(TableAction::Lookup(mut lookup)) => (
            lookup.recv_finished(
                work_storage.announce_port,
                &mut work_storage.routing_table,
                &work_storage.message_out,
            ).await,
            lookup.info_hash(),
        ),
        _ => {
            error!(
                "bittorrent-protocol_dht: Resolved a TransactionID to a check table lookup..."
            );
            return;
        }
    };

    //这里其实只有两种状态：完成或者注册失败
    match lookup_info {
        (LookupStatus::Completed, info_hash) => self.broadcast_dht_event(
            DhtEvent::LookupCompleted(info_hash)
        ).await,
        (LookupStatus::announceFailed, info_hash) => {
            self.broadcast_dht_event(DhtEvent::LookupAnFail(info_hash)).await;
            self.handle_command_shutdown(ShutdownCause::Unspecified).await
        }
        _ => (),
    }
}

}
