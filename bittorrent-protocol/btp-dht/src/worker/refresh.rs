use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::mpsc::Sender;

use btp_util::bt::{self, NodeId};

use crate::message::find_node::FindNodeRequest;
use crate::routing::node::NodeStatus;
use crate::routing::table::{self, RoutingTable};
use crate::transaction::MIDGenerator;
use crate::worker::ScheduledTask;
use btp_util::timer::{Timeout, Timer};

const REFRESH_INTERVAL_TIMEOUT: Duration = Duration::from_millis(6000);
const REFRESH_0_TIMEOUT: Duration = Duration::from_millis(3000);

const REFRESH_CONCURRENCY: usize = 4;

#[derive(Eq, PartialEq)]
pub enum RefreshStatus {
    /// Refresh is in progress.
    Refreshing,
    /// Refresh failed in a fatal way.
    Failed,
}

pub struct TableRefresh {
    id_generator: MIDGenerator,
    curr_refresh_bucket: usize,
    send_message_num: u8,
}

impl TableRefresh {
    pub fn new(id_generator: MIDGenerator) -> TableRefresh {
        TableRefresh {
            id_generator: id_generator,
            curr_refresh_bucket: 0,
            send_message_num: 0,
        }
    }

    pub(crate) async fn continue_refresh(
        &mut self,
        table: &mut RoutingTable,
        out: &Sender<(Vec<u8>, SocketAddr)>,
        timer: &mut Timer<ScheduledTask>,
    ) -> RefreshStatus

    {

        if self.refresh_next_bucket(table, out).await == RefreshStatus::Failed {
            return RefreshStatus::Failed;
        }

        self.curr_refresh_bucket += 1;

        // Generate a dummy transaction id (only the action id will be used)
        let trans_id = self.id_generator.generate();
        if self.send_message_num != 0 {
            // Start a timer for the next refresh
            timer.schedule_in(REFRESH_INTERVAL_TIMEOUT.into(), ScheduledTask::CheckTableRefresh(trans_id));
        }else {
            timer.schedule_in(REFRESH_0_TIMEOUT.into(), ScheduledTask::CheckTableRefresh(trans_id));
        }
        return RefreshStatus::Refreshing;

    }

    async fn refresh_next_bucket(&mut self, table: &mut RoutingTable, out: &Sender<(Vec<u8>, SocketAddr)>) -> RefreshStatus
    {
        self.send_message_num = 0;

        if self.curr_refresh_bucket == table::MAX_BUCKETS {
            self.curr_refresh_bucket = 0;
        }
        let target_id = flip_id_bit_at_index(table.node_id(), self.curr_refresh_bucket);

        info!(
            "bittorrent-protocol_dht: Performing a refresh for bucket {}",
            self.curr_refresh_bucket
        );
        // Ping the closest questionable node
        let nodes = table
            .closest_nodes(target_id)
            .filter(|n| n.status() == NodeStatus::Questionable)
            .filter(|n| !n.recently_requested_from())
            .take(REFRESH_CONCURRENCY)
            .map(|node| *node.handle())
            .collect::<Vec<_>>();

        log::trace!("Refresh nodes num:{:?}",nodes.len());

        for node in nodes
        {
            // Generate a transaction id for the request
            let trans_id = self.id_generator.generate();

            // Construct the message
            let find_node_req = FindNodeRequest::new(trans_id.as_ref(), table.node_id(), target_id);
            let find_node_msg = find_node_req.encode();

            // Send the message
            if out.send((find_node_msg, node.addr)).await.is_err() {
                error!(
                    "bittorrent-protocol_dht: TableRefresh failed to send a refresh message to the out \
                        channel..."
                );
                return RefreshStatus::Failed;
            }

            // Mark that we requested from the node
            // Mark that we requested from the node
            if let Some(node) = table.find_node_mut(&node) {
                node.local_request();
            }
            self.send_message_num += 1;
        }

        RefreshStatus::Refreshing
    }


}
/// Panics if index is out of bounds.
fn flip_id_bit_at_index(node_id: NodeId, index: usize) -> NodeId {
    let mut id_bytes: [u8; bt::NODE_ID_LEN] = node_id.into();
    let (byte_index, bit_index) = (index / 8, index % 8);

    let actual_bit_index = 7 - bit_index;
    id_bytes[byte_index] ^= 1 << actual_bit_index;

    id_bytes.into()
}
