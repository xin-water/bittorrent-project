use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::sync::mpsc::Sender;


use btp_util::bt::{self, InfoHash, NodeId};
use btp_util::net;
use btp_util::sha::ShaHash;

use crate::message::announce_peer::{AnnouncePeerRequest, ConnectPort};
use crate::message::get_peers::{CompactInfoType, GetPeersRequest, GetPeersResponse};
use crate::routing::bucket;
use crate::routing::node::{Node, NodeHandle, NodeStatus};
use crate::routing::table::RoutingTable;
use crate::transaction::{MIDGenerator, TransactionID};
use crate::worker::ScheduledTask;
use crate::worker::timer::{Timeout, Timer};

const LOOKUP_TIMEOUT_MS: Duration = Duration::from_millis(1500);
const ENDGAME_TIMEOUT_MS: Duration = Duration::from_millis(1500);

// Currently using the aggressive variant of the standard lookup procedure.
// https://people.kth.se/~rauljc/p2p11/jimenez2011subsecond.pdf

// TODO: Handle case where a request round fails, should we fail the whole lookup (clear acvite lookups?)
// TODO: Clean up the code in this module.

const INITIAL_PICK_NUM: usize = 4; // Alpha
const ITERATIVE_PICK_NUM: usize = 3; // Beta
const ANNOUNCE_PICK_NUM: usize = 8; // # Announces

type Distance = ShaHash;
type DistanceToBeat = ShaHash;

#[derive(Debug, PartialEq, Eq)]
pub enum LookupStatus {
    Searching,
    Completed,
    Failed,
    announceFailed,
}

pub struct TableLookup {
    table_id: NodeId,
    target_id: InfoHash,
    in_endgame: bool,
    // If we have received any values in the lookup.
    recv_values: bool,
    id_generator: MIDGenerator,
    will_announce: bool,
    // DistanceToBeat is the distance that the responses of the current lookup needs to beat,
    // interestingly enough (and super important), this distance may not be eqaul to the
    // requested node's distance
    active_lookups: HashMap<TransactionID, (DistanceToBeat, Timeout)>,
    announce_tokens: HashMap<NodeHandle, Vec<u8>>,
    requested_nodes: HashSet<NodeHandle>,
    // Storing whether or not it has ever been pinged so that we
    // can perform the brute force lookup if the lookup failed
    all_sorted_nodes: Vec<(Distance, NodeHandle, bool)>,
    tx: mpsc::UnboundedSender<SocketAddr>,
}

// Gather nodes

impl TableLookup {
    pub(crate) async fn new(
        table_id: NodeId,
        target_id: InfoHash,
        id_generator: MIDGenerator,
        will_announce: bool,
        table: &mut RoutingTable,
        out: &mpsc::Sender<(Vec<u8>,SocketAddr)>,
        tx: mpsc::UnboundedSender<SocketAddr>,
        timer: &mut Timer<ScheduledTask>,
    ) -> Option<TableLookup>

    {
        // Pick a buckets worth of nodes and put them into the all_sorted_nodes list
        let mut all_sorted_nodes = Vec::with_capacity(bucket::MAX_BUCKET_SIZE);
        for node in table
            .closest_nodes(target_id)
            .filter(|n| n.status() == NodeStatus::Good)
            .take(bucket::MAX_BUCKET_SIZE)
        {
            insert_sorted_node(&mut all_sorted_nodes, target_id, *node.handle(), false);
        }

        // Call pick_initial_nodes with the all_sorted_nodes list as an iterator
        // 拿取节点，同时修改两个队列中节点的状态
        let initial_pick_nodes = pick_initial_nodes(all_sorted_nodes.iter_mut());
        let initial_pick_nodes_filtered =
            initial_pick_nodes
                .iter()
                .filter(|(_, good)| *good)
                .map(|(node, _)| {
                    let distance_to_beat = node.id ^ target_id;

                    (node, distance_to_beat)
                });

        // Construct the lookup table structure
        let mut table_lookup = TableLookup {
            table_id: table_id,
            target_id: target_id,
            in_endgame: false,
            recv_values: false,
            id_generator: id_generator,
            will_announce: will_announce,
            all_sorted_nodes: all_sorted_nodes,
            announce_tokens: HashMap::new(),
            requested_nodes: HashSet::new(),
            active_lookups: HashMap::with_capacity(INITIAL_PICK_NUM),
            tx: tx,
        };

        // Call start_request_round with the list of initial_nodes (return even if the search completed...for now :D)
        // 启动的时候只要不是Searching状态，都算失败。后面定时器触发时直接拿不到lookup对象，跳过就行。
        if table_lookup.start_request_round(initial_pick_nodes_filtered, table, out, timer).await
            == LookupStatus::Searching
        {
            Some(table_lookup)
        } else {
            None
        }
    }

    pub fn info_hash(&self) -> InfoHash {
        self.target_id
    }

    pub(crate) async fn recv_response<'a>(
        &mut self,
        node: Node,
        trans_id: &TransactionID,
        msg: GetPeersResponse<'a>,
        table: &mut RoutingTable,
        out: &Sender<(Vec<u8>, SocketAddr)>,
        timer: &mut Timer<ScheduledTask>,
    ) -> LookupStatus
    {
        // Process the message transaction id
        let (dist_to_beat, timeout) = if let Some(lookup) = self.active_lookups.remove(trans_id) {
            lookup
        } else {
            warn!(
                "lookup_recv_response: Received lookup node response trans_id:{:?}, not in active_lookup list ...",
                 trans_id
            );
            //可以return 搜索状态，同超时处理。
            return self.current_lookup_status();
        };
        log::trace!( "lookup_recv_response: remove trans_id :{:?}",trans_id);

        // Cancel the timeout (if this is not an endgame response)
        if !self.in_endgame {
            timer.cancel(timeout);
        }

        // Add the announce token to our list of tokens
        if let Some(token) = msg.token() {
            self.announce_tokens.insert(*node.handle(), token.to_vec());
        }

        // Pull out the contact information from the message
        let (opt_values, opt_nodes) = match msg.info_type() {
            CompactInfoType::Nodes(n) => (None, Some(n)),
            CompactInfoType::Values(v) => {
                self.recv_values = true;
                (Some(v.into_iter().collect::<Vec<SocketAddrV4>>()), None)
            }
            CompactInfoType::Both(n, v) => (Some(v.into_iter().collect::<Vec<SocketAddrV4>>()), Some(n)),
        };

        // Check if we beat the distance, get the next distance to beat

        let mut send_iterate_nodes: Option<[(NodeHandle,bool);ITERATIVE_PICK_NUM]> = None;
        let mut next_dist_to_beat = dist_to_beat;

        if let Some(nodes) = opt_nodes {

                let requested_nodes = &self.requested_nodes;

                //接收到的、未发送过请求的节点
                let nodehandles=nodes
                    .into_iter()
                    .map(|(id,addr)|{
                        NodeHandle::new(id, SocketAddr::V4(addr))
                    })
                    .filter(|nodehandle|{
                        !requested_nodes.contains(nodehandle)
                    })
                    .collect::<Vec<NodeHandle>>();

                if nodehandles.len() != 0 {
                    log::trace!("找到新节点:{:?}个",nodehandles.len());
                    // Get the closest distance (or the current distance)
                    let nood_dist_to_beat :DistanceToBeat= nodehandles
                        .iter()
                        .fold(
                            dist_to_beat,
                            |closest, nodehandle| {
                                let distance = self.target_id ^ nodehandle.id;

                                if distance < closest {
                                    distance
                                } else {
                                    closest
                                }
                            },
                        );

                    if nood_dist_to_beat >= dist_to_beat {

                        // 如果新节点距离 比 返回响应的节点距离还远，那就直接插入队列，插入的时候是按距离排列的
                        // Push nodes into the all nodes list
                        for nodehandle in nodehandles {
                            insert_sorted_node(&mut self.all_sorted_nodes, self.target_id, nodehandle, false);
                        }
                        //send_iterate_nodes = None;
                        //next_dist_to_beat = dist_to_beat;
                    } else {

                        // 距离更近，提取未发送过的、最短3个节点，
                        // 数组容量是3个，但可能未发送的节点不到3个，多余的位置是默认地址，状态为false.
                        // 对他们发送查找请求。
                        let iterate_nodes = pick_iterate_nodes(
                            nodehandles
                                .iter()
                                .copied(),
                            self.target_id,
                        );
                        log::trace!("提取{:?}个节点准备发起请求",iterate_nodes.len());

                        // Push nodes into the all nodes list
                        for nodehandle in nodehandles {
                            //判断它是否为发送节点
                            let will_ping = iterate_nodes
                                .iter()
                                .any(|(n, _)| {
                                    *n == nodehandle
                                });

                            // 按距离插入队列，设置即将发送的节点标志为true
                            insert_sorted_node(&mut self.all_sorted_nodes, self.target_id, nodehandle, will_ping);
                        }
                        send_iterate_nodes = Some(iterate_nodes);
                        next_dist_to_beat = nood_dist_to_beat;
                    };

                }

            }


        // Check if we need to iterate (not in the endgame already)
        if !self.in_endgame {
            // If the node gave us a closer id than its own to the target id, continue the search
            if let Some(ref nodes) = send_iterate_nodes {
                let filtered_nodes = nodes
                    .into_iter()
                    .filter(|(_, good)| *good)  //可能取最优节点的时候不到3个，有空位，排除。
                    .map(|(n, _)| (n, next_dist_to_beat));

                if self.start_request_round(filtered_nodes, table, out, timer).await
                    == LookupStatus::Failed
                {
                    //只关注失败情况，其他状态最后一起判断。
                    return LookupStatus::Failed;
                }
            }

            // If there are not more active lookups, start the endgame
            if self.active_lookups.is_empty() {
                if self.start_endgame_round(table, out, timer).await == LookupStatus::Failed {
                    //只关注失败情况，其他状态最后一起判断。
                    return LookupStatus::Failed;
                }
            }
        }

        if let Some(values) = opt_values {
            for v4_addr in values {
                let sock_addr = SocketAddr::V4(v4_addr);
                if self.tx.send(sock_addr).is_err(){
                    log::error!("peer:{:?}发送失败",sock_addr.to_string());
                }
            }
        }

        //同时判断多种情况
        self.current_lookup_status()
    }

    // 此函数只负责移除超时任务和判断开启最后广播，所以状态应该只有失败与继续两种，
    // 没必要调用current_lookup_status，浪费时间，增加复杂度。
    pub(crate) async fn recv_timeout(
        &mut self,
        trans_id: &TransactionID,
        table: &mut RoutingTable,
        out: &mpsc::Sender<(Vec<u8>,SocketAddr)>,
        timer: &mut Timer<ScheduledTask>,
    ) -> LookupStatus

    {
        if self.active_lookups.remove(trans_id).is_none() {
            warn!(
                "lookup_recv_timeout: trans_id:{:?} in active_lookup list not find....",
                 trans_id
            );
            // 理论上到达此处只有Searching状态，
            // 启动错误与启动时没节点都直接在new方法处理了
            // 而搜索完成状态需要start_endgame_round中开启。
            // 为了稳定还是使用状态判断。
            //return LookupStatus::Searching;
            return self.current_lookup_status();

        }
        log::trace!( "lookup_recv_timeout: remove trans_id :{:?}",trans_id);

        //未开启最后广播，活跃请求为0的时候，开启广播。
        if !self.in_endgame {
            // If there are not more active lookups, start the endgame
            if self.active_lookups.is_empty() {
                if self.start_endgame_round(table, out, timer).await == LookupStatus::Failed {
                    return LookupStatus::Failed;
                }
            }
        }

        // 在这里调用状态方法可以直接判断 第一阶段与endgame启动阶段状态
        // endgame超时不走这里。
        return self.current_lookup_status();
    }

    //最后自己注册到dht节点阶段。注册完lookup对象将被外部函数移除。
    pub async fn recv_finished(
        &mut self,
        announce_port: Option<u16>,
        table: &mut RoutingTable,
        out: &mpsc::Sender<(Vec<u8>,SocketAddr)>,
    ) -> LookupStatus {
        let mut fatal_error = false;

        // Announce if we were told to
        if self.will_announce && announce_port.is_some(){
            // Partial borrow so the filter function doesnt capture all of self
            let announce_tokens = &self.announce_tokens;

            //弹出8个节点进行注册。
            for &(_, ref node, _) in self
                .all_sorted_nodes
                .iter()
                .filter(|(_, node, _)| announce_tokens.contains_key(node))
                .take(ANNOUNCE_PICK_NUM)
            {
                let trans_id = self.id_generator.generate();
                let token = announce_tokens.get(node).unwrap();

                let announce_peer_req = AnnouncePeerRequest::new(
                    trans_id.as_ref(),
                    self.table_id,
                    self.target_id,
                    token.as_ref(),
                    ConnectPort::Explicit(announce_port.unwrap()),
                );
                let announce_peer_msg = announce_peer_req.encode();

                if out.send((announce_peer_msg, node.addr)).await.is_err() {
                    error!(
                        "bittorrent-protocol_dht: TableLookup announce request failed to send through the out \
                            channel..."
                    );

                    fatal_error = true;
                    // 使用socket是跳到下一个循环，
                    // 使用通道是跳出循环，因为通道错误就完全不能发送了。
                    break
                }

                //有错误的时候都不走这里了，所以不用判断了。
                // if !fatal_error {
                //     // We requested from the node, marke it down if the node is in our routing table
                //     table.find_node_mut(node).map(|n| n.local_request());
                // }

                table.find_node_mut(node).map(|n| n.local_request());
                // 为兼容socket有的发送失败，有的发送成功而做，
                // 如果不需要更改为socket，应该发送失败时直接返回注册失败状态，而最后时默认完成就好。
                fatal_error = false;
            }
        }

        // This may not be cleared since we didnt set a timeout for each node, any nodes that didnt respond would still be in here.

        // 如果handler中已经移除了lookup对象，此处在修改状态好像是做无用功了，
        // self.active_lookups.clear();
        // self.in_endgame = false;

        if fatal_error {
            LookupStatus::announceFailed
        } else {
            //上面修改了状态，此处调用应该是返回complet,但既然都任务都终结了，直接返回完成不是更好？
            // self.current_lookup_status()
            LookupStatus::Completed
        }
    }

    fn current_lookup_status(&self) -> LookupStatus {
        if self.in_endgame || !self.active_lookups.is_empty() {
            LookupStatus::Searching
        } else {
            LookupStatus::Completed
        }
    }

    // 第一阶段所有节点请求都从这里发起，如果请求数量为0,可以判断没有相关节点，那么任务应该就直接完成了。
    async fn start_request_round<'a, I>(
        &mut self,
        nodes: I,
        table: &mut RoutingTable,
        out: &mpsc::Sender<(Vec<u8>, SocketAddr)>,
        timer: &mut Timer<ScheduledTask>,
    ) -> LookupStatus
    where
        I: Iterator<Item = (&'a NodeHandle, DistanceToBeat)>,

    {
        // Loop through the given nodes
        let mut messages_sent = 0;
        for (node, dist_to_beat) in nodes {
            // Generate a transaction id for this message
            let trans_id = self.id_generator.generate();

            // Try to start a timeout for the node
            let timeout = timer.schedule_in(LOOKUP_TIMEOUT_MS,ScheduledTask::CheckLookupTimeout(trans_id));
            // Associate the transaction id with the distance the returned nodes must beat and the timeout token
            self.active_lookups
                .insert(trans_id, (dist_to_beat, timeout));

            // Send the message to the node
            let get_peers_msg =
                GetPeersRequest::new(trans_id.as_ref(), self.table_id, self.target_id).encode();
            if out.send((get_peers_msg, node.addr)).await.is_err() {
                error!("bittorrent-protocol_dht: Could not send a lookup message through the channel...");

                // 如果是socket,可以开启下一次循环，因为不一定是传输的原因，可能是数据包过大。
                // 通道发送失败，直接终止，
                return LookupStatus::Failed;
            }
            log::trace!("bittorrent-protocol_dht: start_request_round 向节点：{:?}请求 peer ",node.id);

            // We requested from the node, mark it down
            self.requested_nodes.insert(*node);

            // Update the node in the routing table
            table.find_node_mut(node).map(|n| n.local_request());

            messages_sent += 1;
        }

        // 可以判断noods为0与socket发送失败两种情况。保留。
        if messages_sent == 0 {
            self.active_lookups.clear();
            LookupStatus::Completed
        } else {
            LookupStatus::Searching
        }
    }

    // 判断无peer向 未发送请求节点 广播请求
    // 这个函数可以在发送之前判断请求节点数量，如果数量为0,那么可以直接返回完成状态。
    // 当前算法也是可以的，减少了外层函数状态判断，只是在节点数为0的时候会延长总时间。
    // 估计大部分情况下节点为0属于不可能事件，而且减少外部状态判断，所以保留当前算法。
    async fn start_endgame_round(
        &mut self,
        table: &mut RoutingTable,
        out: &mpsc::Sender<(Vec<u8>, SocketAddr)>,
        timer: &mut Timer<ScheduledTask>,
    ) -> LookupStatus

    {
        // Entering the endgame phase
        self.in_endgame = true;
        warn!("start_endgame_round");
        // Try to start a global message timeout for the endgame
        let timeout = timer.schedule_in(ENDGAME_TIMEOUT_MS,ScheduledTask::CheckLookupEndGame(self.id_generator.generate()));


        // Request all unpinged nodes if we didnt receive any values
        if !self.recv_values {
            for node_info in self
                .all_sorted_nodes
                .iter_mut()
                .filter(|&&mut (_, _, req)| !req)
            {
                let &mut (ref node_dist, ref node, ref mut req) = node_info;

                // Generate a transaction id for this message
                let trans_id = self.id_generator.generate();

                // Associate the transaction id with this node's distance and its timeout token
                // We dont actually need to keep track of this information, but we do still need to
                // filter out unsolicited responses by using the active_lookups map!!!
                self.active_lookups.insert(trans_id, (*node_dist, timeout));

                // Send the message to the node
                let get_peers_msg =
                    GetPeersRequest::new(trans_id.as_ref(), self.table_id, self.target_id).encode();
                if out.send((get_peers_msg, node.addr)).await.is_err() {
                    error!("bittorrent-protocol_dht: Could not send an endgame message through the channel...");

                    // 通道发送错误，直接返回失败
                    // 如果是socket可以跳到下一个循环
                    return LookupStatus::Failed;
                }

                // Mark that we requested from the node in the RoutingTable
                table.find_node_mut(node).map(|n| n.local_request());

                // Mark that we requested from the node
                *req = true;
            }
            // 目前外部函数只专注于失败情况，其他情况它会调用状态判断函数判断，
            // 这里其实只要不是失败都可以，为了完整性，依旧按正常状态返回。
            // 其实可以改进为返回rensult,ok或err两种情况。
            return LookupStatus::Searching;
        }
        //同上
        LookupStatus::Completed
    }
}

//筛选迭代器中节点
/// Picks a number of nodes from the sorted distance iterator to ping on the first round.
fn pick_initial_nodes<'a, I>(sorted_nodes: I) -> [(NodeHandle, bool); INITIAL_PICK_NUM]
where
    I: Iterator<Item = &'a mut (Distance, NodeHandle, bool)>,
{
    let dummy_id = [0u8; bt::NODE_ID_LEN].into();
    // let default = (Node::as_bad(dummy_id, net::default_route_v4()), false);
    // let mut pick_nodes = [
    //     default.clone(),
    //     default.clone(),
    //     default.clone(),
    //     default.clone(),
    // ];

    let default = (
        NodeHandle::new(dummy_id, SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))),
        false,
    );
    let mut pick_nodes = [default; INITIAL_PICK_NUM];


    for (src, dst) in sorted_nodes.zip(pick_nodes.iter_mut()) {
        dst.0 = src.1.clone();
        dst.1 = true;

        // Mark that the node has been requested from
        src.2 = true;
    }

    pick_nodes
}

/// Picks a number of nodes from the unsorted distance iterator to ping on iterative rounds.
fn pick_iterate_nodes<I>(
    unsorted_nodes: I,
    target_id: InfoHash,
) -> [(NodeHandle, bool); ITERATIVE_PICK_NUM]
where
    I: Iterator<Item = NodeHandle>,
{
    let dummy_id = [0u8; bt::NODE_ID_LEN].into();

    // let default = (Node::as_bad(dummy_id, net::default_route_v4()), false);
    // let mut pick_nodes = [default.clone(), default.clone(), default.clone()];
    let default = (
        NodeHandle::new(dummy_id, SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))),
        false,
    );
    let mut pick_nodes = [default; ITERATIVE_PICK_NUM];

    for nodehandle in unsorted_nodes {
        insert_closest_nodes(&mut pick_nodes, target_id, nodehandle);
    }

    pick_nodes
}

//替换迭代器中节点,状态为false的或距离远的，设置状态为true
/// Inserts the node into the slice if a slot in the slice is unused or a node
/// in the slice is further from the target id than the node being inserted.
fn insert_closest_nodes(nodes: &mut [(NodeHandle, bool)], target_id: InfoHash, new_node: NodeHandle) {
    let new_distance = target_id ^ new_node.id;

    for &mut (ref mut old_node, ref mut used) in nodes.iter_mut() {
        if !*used {
            // Slot was not in use, go ahead and place the node
            *old_node = new_node;
            *used = true;
            return;
        } else {
            // Slot is in use, see if our node is closer to the target
            let old_distance = target_id ^ old_node.id;

            if new_distance < old_distance {
                *old_node = new_node;
                return;
            }
        }
    }
}

/// Inserts the Node into the list of nodes based on its distance from the target node.
///
/// Nodes at the start of the list are closer to the target node than nodes at the end.
fn insert_sorted_node(
    nodes: &mut Vec<(Distance, NodeHandle, bool)>,
    target: InfoHash,
    node: NodeHandle,
    pinged: bool,
) {
    let node_id = node.id;
    let node_dist = target ^ node_id;

    // Perform a search by distance from the target id
    let search_result = nodes.binary_search_by(|&(dist, _, _)| dist.cmp(&node_dist));
    match search_result {
        Ok(dup_index) => {
            // TODO: Bug here, what happens when multiple nodes with the same distance are
            // present, but we dont get the index of the duplicate node (its in the list) from
            // the search, then we would have a duplicate node in the list!
            // Insert only if this node is different (it is ok if they have the same id)
            if nodes[dup_index].1 != node {
                nodes.insert(dup_index, (node_dist, node, pinged));
            }
        }
        Err(ins_index) => nodes.insert(ins_index, (node_dist, node, pinged)),
    };
}
