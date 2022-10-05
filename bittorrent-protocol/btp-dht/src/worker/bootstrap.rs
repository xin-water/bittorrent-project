use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Duration;

use crate::message::find_node::FindNodeRequest;
use crate::routing::bucket::Bucket;
use crate::routing::node::{Node, NodeStatus};
use crate::routing::table::{self, BucketContents, RoutingTable};
use crate::transaction::{MIDGenerator, TransactionID};
use crate::worker::handler::DhtHandler;
use crate::worker::ScheduledTask;
use btp_util::bt::{self, NodeId};
use crate::worker::socket::Socket;
use crate::worker::timer::{Timeout, Timer};

const BOOTSTRAP_INITIAL_TIMEOUT: Duration = Duration::from_millis(2500);
const BOOTSTRAP_NODE_TIMEOUT: Duration = Duration::from_millis(500);

const BOOTSTRAP_PINGS_PER_BUCKET: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapStatus {
    /// Bootstrap has been finished.
    Idle,
    /// Bootstrap is in progress.
    Bootstrapping,
    /// Bootstrap just finished.
    Completed,
    /// Bootstrap failed in a fatal way.
    Failed,
}

pub struct TableBootstrap {
    table_id: NodeId,
    id_generator: MIDGenerator,
    starting_nodes: Vec<SocketAddr>,
    active_messages: HashMap<TransactionID, Timeout>,
    starting_routers: HashSet<SocketAddr>,
    curr_bootstrap_bucket: usize,
}

impl TableBootstrap {
    pub fn new<I>(
        table_id: NodeId,
        id_generator: MIDGenerator,
        nodes: Vec<SocketAddr>,
        routers: I,
    ) -> TableBootstrap
    where
        I: Iterator<Item = SocketAddr>,
    {
        let router_filter: HashSet<SocketAddr> = routers.collect();

        TableBootstrap {
            table_id: table_id,
            id_generator: id_generator,
            starting_nodes: nodes,
            starting_routers: router_filter,
            active_messages: HashMap::new(),
            curr_bootstrap_bucket: 0,
        }
    }

    pub async fn start_bootstrap(
        &mut self,
        out: &Socket,
        timer: &mut Timer<ScheduledTask>,
    ) -> BootstrapStatus

    {
        // Reset the bootstrap state
        self.active_messages.clear();
        self.curr_bootstrap_bucket = 0;

        // Generate transaction id for the initial bootstrap messages
        let trans_id = self.id_generator.generate();

        // Set a timer to begin the actual bootstrap
        let timeout = timer.schedule_in(BOOTSTRAP_INITIAL_TIMEOUT,ScheduledTask::CheckBootstrapTimeout(trans_id));

        // Insert the timeout into the active bootstraps just so we can check if a response was valid (and begin the bucket bootstraps)
        self.active_messages.insert(trans_id, timeout);

        let find_node_msg =
            FindNodeRequest::new(trans_id.as_ref(), self.table_id, self.table_id).encode();
        // Ping all initial routers and nodes
        for addr in self
            .starting_routers
            .iter()
            .chain(self.starting_nodes.iter())
        {
            if out.send(&find_node_msg[..], *addr).await.is_err() {
                error!("bittorrent-protocol_dht: Failed to send bootstrap message to router through channel...");
                return BootstrapStatus::Failed;
            }
        }

        self.current_bootstrap_status()
    }

    pub fn is_router(&self, addr: &SocketAddr) -> bool {
        self.starting_routers.contains(&addr)
    }

    pub async fn recv_response<'a>(
        &mut self,
        trans_id: &TransactionID,
        table: &RoutingTable,
        //out: &SyncSender<(Vec<u8>, SocketAddr)>,
        out: &Socket,
        timer: &mut Timer<ScheduledTask>,
    ) -> BootstrapStatus

    {
        // Process the message transaction id
        let timeout = if let Some(t) = self.active_messages.get(trans_id) {
            *t
        } else {
            warn!(
                "dht: Received bootstrap node response timeout, not in active_messages list ..."
            );
            return self.current_bootstrap_status();
        };

        // If this response was from the initial bootstrap, we don't want to clear the timeout or remove
        // the token from the map as we want to wait until the proper timeout has been triggered before starting
        if self.curr_bootstrap_bucket != 0 {
            // Message was not from the initial ping
            // Remove the timeout from the event loop
            timer.cancel(timeout);

            // Remove the token from the mapping
            self.active_messages.remove(trans_id);
        }

        // Check if we need to bootstrap on the next bucket
        if self.active_messages.is_empty() {
            return self.bootstrap_next_bucket(table, out, timer).await;
        }

        self.current_bootstrap_status()
    }

    pub async fn recv_timeout(
        &mut self,
        trans_id: &TransactionID,
        table: &RoutingTable,
        out: &Socket,
        timer: &mut Timer<ScheduledTask>,
    ) -> BootstrapStatus

    {
        if self.active_messages.remove(trans_id).is_none() {
            warn!(
                "dht: bootstrap is response, active_message list not find...."
            );
            return self.current_bootstrap_status();
        }

        // Check if we need to bootstrap on the next bucket
        if self.active_messages.is_empty() {
            return self.bootstrap_next_bucket(table, out, timer).await;
        }

        self.current_bootstrap_status()
    }

    // Returns true if there are more buckets to bootstrap, false otherwise
    async fn bootstrap_next_bucket(
        &mut self,
        table: &RoutingTable,
        out: &Socket,
        timer: &mut Timer<ScheduledTask>,
    ) -> BootstrapStatus

    {
        let target_id = flip_id_bit_at_index(self.table_id, self.curr_bootstrap_bucket);

        // Get the optimal iterator to bootstrap the current bucket
        if self.curr_bootstrap_bucket == 0 || self.curr_bootstrap_bucket == 1 {
            let iter = table
                .closest_nodes(target_id)
                .filter(|n| n.status() == NodeStatus::Questionable);

            self.send_bootstrap_requests(iter, target_id, table, out, timer).await
        } else {
            let mut buckets = table.buckets().skip(self.curr_bootstrap_bucket - 2);
            let dummy_bucket = Bucket::new();

            // Sloppy probabilities of our target node residing at the node
            let percent_25_bucket = if let Some(bucket) = buckets.next() {
                match bucket {
                    BucketContents::Empty => dummy_bucket.iter(),
                    BucketContents::Sorted(b) => b.iter(),
                    BucketContents::Assorted(b) => b.iter(),
                }
            } else {
                dummy_bucket.iter()
            };
            let percent_50_bucket = if let Some(bucket) = buckets.next() {
                match bucket {
                    BucketContents::Empty => dummy_bucket.iter(),
                    BucketContents::Sorted(b) => b.iter(),
                    BucketContents::Assorted(b) => b.iter(),
                }
            } else {
                dummy_bucket.iter()
            };
            let percent_100_bucket = if let Some(bucket) = buckets.next() {
                match bucket {
                    BucketContents::Empty => dummy_bucket.iter(),
                    BucketContents::Sorted(b) => b.iter(),
                    BucketContents::Assorted(b) => b.iter(),
                }
            } else {
                dummy_bucket.iter()
            };

            // TODO: Figure out why chaining them in reverse gives us more total nodes on average, perhaps it allows us to fill up the lower
            // buckets faster at the cost of less nodes in the higher buckets (since lower buckets are very easy to fill)...Although it should
            // even out since we are stagnating buckets, so doing it in reverse may make sense since on the 3rd iteration, it allows us to ping
            // questionable nodes in our first buckets right off the bat.
            let iter = percent_25_bucket
                .chain(percent_50_bucket)
                .chain(percent_100_bucket)
                .filter(|n| n.status() == NodeStatus::Questionable);

            self.send_bootstrap_requests(iter, target_id, table, out, timer).await
        }
    }

    async fn send_bootstrap_requests<'a, I>(
        &mut self,
        nodes: I,
        target_id: NodeId,
        table: &RoutingTable,
        out: &Socket,
        timer: &mut Timer<ScheduledTask>,
    ) -> BootstrapStatus
    where
        I: Iterator<Item = &'a Node>,
    {
        info!(
            "bittorrent-protocol_dht: bootstrap::send_bootstrap_requests {}",
            self.curr_bootstrap_bucket
        );

        let mut messages_sent = 0;

        for node in nodes.take(BOOTSTRAP_PINGS_PER_BUCKET) {
            // Generate a transaction id
            let trans_id = self.id_generator.generate();
            let find_node_msg =
                FindNodeRequest::new(trans_id.as_ref(), self.table_id, target_id).encode();

            // Add a timeout for the node
            let timeout = timer.schedule_in(BOOTSTRAP_NODE_TIMEOUT,ScheduledTask::CheckBootstrapTimeout(trans_id));

            // Send the message to the node
            if out.send(&find_node_msg[..], node.addr()).await.is_err() {
                error!("bittorrent-protocol_dht: Could not send a bootstrap message through the channel...");
                return BootstrapStatus::Failed;
            }

            // Mark that we requested from the node
            node.local_request();

            // Create an entry for the timeout in the map
            self.active_messages.insert(trans_id, timeout);

            messages_sent += 1;
        }

        self.curr_bootstrap_bucket += 1;
        if self.curr_bootstrap_bucket == table::MAX_BUCKETS {
            return BootstrapStatus::Completed;
        } else if messages_sent == 0 {
            self.bootstrap_next_bucket(table, out, timer).await
        } else {
            return BootstrapStatus::Bootstrapping;
        }
    }

    fn current_bootstrap_status(&self) -> BootstrapStatus {
        if self.curr_bootstrap_bucket == table::MAX_BUCKETS || self.active_messages.is_empty() {
            BootstrapStatus::Idle
        } else {
            BootstrapStatus::Bootstrapping
        }
    }
}

/// Panics if index is out of bounds.
/// TODO: Move into use btp_util crate
fn flip_id_bit_at_index(node_id: NodeId, index: usize) -> NodeId {
    let mut id_bytes: [u8; bt::NODE_ID_LEN] = node_id.into();
    let (byte_index, bit_index) = (index / 8, index % 8);

    let actual_bit_index = 7 - bit_index;
    id_bytes[byte_index] ^= 1 << actual_bit_index;

    id_bytes.into()
}
