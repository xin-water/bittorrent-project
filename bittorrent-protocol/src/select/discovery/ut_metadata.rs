use std::collections::HashMap;
use std::collections::HashSet;
// use std::collections::VecDeque;
use bytes::BytesMut;
use rand::{self, Rng};
use std::collections::hash_map::Entry;
use std::collections::vec_deque::VecDeque;
use std::io::Write;
use std::time::Duration;

use crate::handshake::InfoHash;
use crate::metainfo::Info;
use crate::metainfo::Metainfo;
use crate::peer::messages::builders::ExtendedMessageBuilder;
use crate::peer::messages::UtMetadataDataMessage;
use crate::peer::messages::UtMetadataMessage;
use crate::peer::messages::UtMetadataRejectMessage;
use crate::peer::messages::UtMetadataRequestMessage;
use crate::peer::messages::{ExtendedMessage, ExtendedType};
use crate::peer::PeerInfo;

use crate::select::discovery::error::{DiscoveryError, DiscoveryErrorKind};
use crate::select::discovery::{IDiscoveryMessage, ODiscoveryMessage, Run};
use crate::select::extended::{ExtendedListener, ExtendedPeerInfo};
use crate::select::ControlMessage;

const REQUEST_TIMEOUT_MILLIS: u64 = 5000;
const MAX_REQUEST_SIZE: usize = 16 * 1024;

const MAX_ACTIVE_REQUESTS: usize = 100;
const MAX_PEER_REQUESTS: usize = 100;

/// Module for sending/receiving metadata from other peers.
///
/// If you are using this module, you should make sure to handshake
/// peers with `Extension::ExtensionProtocol` active. Failure to do
/// this will result in this module not sending any messages.
///
/// Metadata will be retrieved when `IDiscoveryMessage::DownloadMetadata`
/// is received, and will be served when
/// `IDiscoveryMessage::Control(ControlMessage::AddTorrent)` is received.
pub struct UtMetadataModule {
    //已完成下载的种子列表
    completed_map: HashMap<InfoHash, Vec<u8>>,

    //未完成下载的种子列表
    pending_map: HashMap<InfoHash, Option<PendingInfo>>,

    //正在传输的种子片段
    active_requests: Vec<ActiveRequest>,

    //正在传输种子片段的对等点
    active_peers: HashMap<InfoHash, ActivePeers>,

    //保存其他人向我发起的请求
    peer_requests: VecDeque<PeerRequest>,
}

struct PendingInfo {
    messages: Vec<UtMetadataRequestMessage>,
    left: usize,
    bytes: Vec<u8>,
}

struct ActiveRequest {
    left: Duration,
    message: UtMetadataRequestMessage,
    sent_to: PeerInfo,
}

struct PeerRequest {
    send_to: PeerInfo,
    request: UtMetadataRequestMessage,
}

struct ActivePeers {
    peers: HashSet<PeerInfo>,
    metadata_size: i64,
}

impl UtMetadataModule {
    /// Create a new `UtMetadataModule`.
    pub fn new() -> UtMetadataModule {
        UtMetadataModule {
            completed_map: HashMap::new(),
            pending_map: HashMap::new(),
            active_peers: HashMap::new(),
            active_requests: Vec::new(),
            peer_requests: VecDeque::new(),
        }
    }

    fn add_torrent(
        &mut self,
        metainfo: Metainfo,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        let info_hash = metainfo.info().info_hash();

        match self.completed_map.entry(info_hash) {
            Entry::Occupied(_) => Err(DiscoveryError::from_kind(
                DiscoveryErrorKind::InvalidMetainfoExists { hash: info_hash },
            )),
            Entry::Vacant(vac) => {
                let info_bytes = metainfo.info().to_bytes();
                vac.insert(info_bytes);

                Ok(None)
            }
        }
    }

    fn remove_torrent(
        &mut self,
        metainfo: Metainfo,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        if self
            .completed_map
            .remove(&metainfo.info().info_hash())
            .is_none()
        {
            Err(DiscoveryError::from_kind(
                DiscoveryErrorKind::InvalidMetainfoNotExists {
                    hash: metainfo.info().info_hash(),
                },
            ))
        } else {
            Ok(None)
        }
    }

    fn add_peer(
        &mut self,
        info: PeerInfo,
        ext_info: &ExtendedPeerInfo,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        let our_support = ext_info
            .our_message()
            .and_then(|msg| msg.query_id(&ExtendedType::UtMetadata))
            .is_some();
        let they_support = ext_info
            .their_message()
            .and_then(|msg| msg.query_id(&ExtendedType::UtMetadata))
            .is_some();
        let opt_metadata_size = ext_info
            .their_message()
            .and_then(ExtendedMessage::metadata_size);

        info!(
            "Our Support For UtMetadata Is {:?} And {:?} Support For UtMetadata Is {:?} With Metdata Size {:?}",
            our_support,
            info.addr(),
            they_support,
            opt_metadata_size
        );
        // If peer supports it, but they dont have the metadata size, then they probably dont have the file yet...
        match (our_support, they_support, opt_metadata_size) {
            (true, true, Some(metadata_size)) => {
                self.active_peers
                    .entry(*info.hash())
                    .or_insert_with(|| ActivePeers {
                        peers: HashSet::new(),
                        metadata_size: metadata_size,
                    })
                    .peers
                    .insert(info);
            }
            _ => (),
        }

        Ok(None)
    }

    fn remove_peer(&mut self, info: PeerInfo) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        let empty_peers = if let Some(active_peers) = self.active_peers.get_mut(info.hash()) {
            active_peers.peers.remove(&info);

            active_peers.peers.is_empty()
        } else {
            false
        };

        if empty_peers {
            self.active_peers.remove(&info.hash());
        }

        Ok(None)
    }

    fn apply_tick(
        &mut self,
        duration: Duration,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        let active_requests = &mut self.active_requests;
        let active_peers = &mut self.active_peers;
        let pending_map = &mut self.pending_map;

        // Retain only the requests that arent expired
        active_requests.retain(|request| {
            let is_expired = request.left.checked_sub(duration).is_none();
            //info!("[apply_tick] {:?},left:{:?},is_expired:{:?}",request.message.piece(),request.left,is_expired);
            if is_expired {
                // Peer didnt respond to our request, remove from active peers
                if let Some(active) = active_peers.get_mut(&request.sent_to.hash()) {
                    active.peers.remove(&request.sent_to);
                }

                // Push request back to pending
                pending_map
                    .get_mut(&request.sent_to.hash())
                    .map(|opt_pending| {
                        opt_pending.as_mut().map(|pending| {
                            pending.messages.push(request.message);
                        })
                    });
            }

            !is_expired
        });

        // Go back through and subtract from the left over requests, they wont underflow
        for active_request in active_requests.iter_mut() {
            active_request.left -= duration;
        }

        //info!("[apply_tick] active_requests len:{:?}\n",&self.active_requests.len());
        Ok(None)
    }

    fn download_metainfo(
        &mut self,
        hash: InfoHash,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        if !self.pending_map.contains_key(&hash) {
            self.pending_map.insert(hash, None);
        }

        Ok(None)
    }

    fn recv_request(
        &mut self,
        info: PeerInfo,
        request: UtMetadataRequestMessage,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        if self.peer_requests.len() == MAX_PEER_REQUESTS {
            Ok(Some(IDiscoveryMessage::ReceivedUtMetadataMessage(
                info,
                UtMetadataMessage::Request(request),
            )))
        } else {
            self.peer_requests.push_back(PeerRequest {
                send_to: info,
                request: request,
            });

            Ok(None)
        }
    }

    fn recv_data(
        &mut self,
        info: PeerInfo,
        data: UtMetadataDataMessage,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        // See if we can find the request that we made to the peer for that piece
        let opt_index = self
            .active_requests
            .iter()
            .position(|request| request.sent_to == info && request.message.piece() == data.piece());

        // If so, go ahead and process it, if not, ignore it (could ban peer...)
        if let Some(index) = opt_index {
            self.active_requests.swap_remove(index);

            if let Some(&mut Some(ref mut pending)) = self.pending_map.get_mut(&info.hash()) {
                let data_offset = (data.piece() as usize) * MAX_REQUEST_SIZE;

                pending.left -= 1;
                (&mut pending.bytes.as_mut_slice()[data_offset..])
                    .write(data.data().as_ref())
                    .unwrap();
            }
        }

        Ok(None)
    }

    fn recv_reject(
        &mut self,
        _info: PeerInfo,
        _reject: UtMetadataRejectMessage,
    ) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        // TODO: Remove any requests after receiving a reject, for now, we will just timeout
        Ok(None)
    }

    //-------------------------------------------------------------------------------//

    fn retrieve_completed_download(&mut self) -> Option<Result<ODiscoveryMessage, DiscoveryError>> {
        let opt_completed_hash = self
            .pending_map
            .iter()
            .find(|&(_, ref opt_pending)| {
                opt_pending
                    .as_ref()
                    .map(|pending| pending.left == 0)
                    .unwrap_or(false)
            })
            .map(|(hash, _)| *hash);

        opt_completed_hash.and_then(|completed_hash| {
            let completed = self.pending_map.remove(&completed_hash).unwrap().unwrap();

            // Clean up other structures since the download is complete
            self.active_peers.remove(&completed_hash);

            match Info::from_bytes(&completed.bytes[..]) {
                Ok(info) => Some(Ok(ODiscoveryMessage::DownloadedMetainfo(info.into()))),
                Err(_) => self.retrieve_completed_download(),
            }
        })
    }

    fn retrieve_piece_request(&mut self) -> Option<Result<ODiscoveryMessage, DiscoveryError>> {
        for (hash, opt_pending) in self.pending_map.iter_mut() {

            let has_ready_requests = opt_pending
                .as_ref()
                .map(|pending| !pending.messages.is_empty())
                .unwrap_or(false);

            let has_active_peers = self
                .active_peers
                .get(hash)
                .map(|peers| !peers.peers.is_empty())
                .unwrap_or(false);

            if has_ready_requests && has_active_peers {
                let pending = opt_pending.as_mut().unwrap();

                let mut active_peers = self.active_peers.get(hash).unwrap().peers.iter();
                let num_active_peers = active_peers.len();
                let selected_peer_num = rand::thread_rng().next_u32() as usize % num_active_peers;

                let selected_peer = active_peers.nth(selected_peer_num).unwrap();
                let selected_message = pending.messages.pop().unwrap();

                self.active_requests
                    .push(generate_active_request(selected_message, *selected_peer));

                info!(
                    "Requesting Piece {:?} For Hash {:?}",
                    selected_message.piece(),
                    selected_peer.hash()
                );
                return Some(Ok(ODiscoveryMessage::SendUtMetadataMessage(
                    *selected_peer,
                    UtMetadataMessage::Request(selected_message),
                )));
            }
        }

        None
    }

    fn retrieve_piece_response(&mut self) -> Option<Result<ODiscoveryMessage, DiscoveryError>> {
        while let Some(request) = self.peer_requests.pop_front() {
            let hash = request.send_to.hash();
            let piece = request.request.piece();

            let start = piece as usize * MAX_REQUEST_SIZE;
            let end = start + MAX_REQUEST_SIZE;

            if let Some(data) = self.completed_map.get(hash) {
                if start <= data.len() && end <= data.len() {
                    let info_slice: &[u8] = &data[start..end];
                    let mut info_payload = BytesMut::with_capacity(info_slice.len());

                    info_payload.extend_from_slice(info_slice);
                    let message = UtMetadataDataMessage::new(
                        piece,
                        info_slice.len() as i64,
                        info_payload.freeze(),
                    );

                    return Some(Ok(ODiscoveryMessage::SendUtMetadataMessage(
                        request.send_to,
                        UtMetadataMessage::Data(message),
                    )));
                } else {
                    // Peer asked for a piece outside of the range...dont respond to that
                }
            }
        }

        None
    }

    //-------------------------------------------------------------------------------//

    fn initialize_pending(&mut self) -> bool {
        let mut pending_tasks_available = false;

        // Initialize PeningInfo once we get peers that have told us the metadata size
        for (hash, opt_pending) in self.pending_map.iter_mut() {
            if opt_pending.is_none() {
                let opt_pending_info = self.active_peers.get(hash).map(|active_peers| {
                    pending_info_from_metadata_size(active_peers.metadata_size)
                });

                *opt_pending = opt_pending_info;
            }

            // If pending is there, and the messages array is not empty
            pending_tasks_available |= opt_pending
                .as_ref()
                .map(|pending| !pending.messages.is_empty())
                .unwrap_or(false);
        }

        pending_tasks_available
    }

    fn validate_downloaded(&mut self) -> bool {
        let mut completed_downloads_available = false;

        // Sweep over all "pending" requests, and check if completed downloads pass hash validation
        // If not, set them back to None so they get re-initialized
        // If yes, mark down that we have completed downloads
        for (&expected_hash, opt_pending) in self.pending_map.iter_mut() {
            let should_reset = opt_pending
                .as_mut()
                .map(|pending| {
                    if pending.left == 0 {
                        let real_hash = InfoHash::from_bytes(&pending.bytes[..]);
                        let needs_reset = real_hash != expected_hash;

                        // If we dont need a reset, we finished and validation passed!
                        completed_downloads_available |= !needs_reset;

                        // If we need a reset, we finished and validation failed!
                        needs_reset
                    } else {
                        false
                    }
                })
                .unwrap_or(false);

            if should_reset {
                *opt_pending = None;
            }
        }

        completed_downloads_available
    }

    //-------------------------------------------------------------------------------//

    fn check_stream_unblock(&mut self) {

        // Will potentially re-initialize downloads that failed hash check
        let tasks_available = self.initialize_pending();
        let free_task_queue_space = self.active_requests.len() != MAX_ACTIVE_REQUESTS;

        let peer_requests_available = !self.peer_requests.is_empty();

        // Will invalidate downloads that dont pass hash check
        let downloads_available = self.validate_downloaded();

        // Check if stream is currently blocked AND either we can queue more requests OR we can service some requests OR we have complete downloads
        // let should_unblock = (
        //     (tasks_available && free_task_queue_space)
        //     || peer_requests_available
        //     || downloads_available);
        //
        // if should_unblock {
        //     info!("UtMetadata 入站信息 已生成任务!");
        // }
    }

    fn check_sink_unblock(&mut self) {
        // Check if sink is currently blocked AND max peer requests has not been reached
        // let should_unblock = self.peer_requests.len() != MAX_PEER_REQUESTS;
        //
        // if should_unblock {
        //    info!("UtMetadata peer_requests 请求队列未满");
        // }
    }
}

fn generate_active_request(message: UtMetadataRequestMessage, peer: PeerInfo) -> ActiveRequest {
    ActiveRequest {
        left: Duration::from_millis(REQUEST_TIMEOUT_MILLIS),
        message: message,
        sent_to: peer,
    }
}

fn pending_info_from_metadata_size(metadata_size: i64) -> PendingInfo {
    let cast_metadata_size = metadata_size as usize;

    let bytes = vec![0u8; cast_metadata_size];
    let mut messages = Vec::new();

    let num_pieces = if cast_metadata_size % MAX_REQUEST_SIZE != 0 {
        cast_metadata_size / MAX_REQUEST_SIZE + 1
    } else {
        cast_metadata_size / MAX_REQUEST_SIZE
    };

    for index in 0..num_pieces {
        messages.push(UtMetadataRequestMessage::new((index) as i64));
    }

    PendingInfo {
        messages: messages,
        left: num_pieces,
        bytes: bytes,
    }
}

//-------------------------------------------------------------------------------//

impl ExtendedListener for UtMetadataModule {
    fn extend(&self, _info: &PeerInfo, builder: ExtendedMessageBuilder) -> ExtendedMessageBuilder {
        builder.with_extended_type(ExtendedType::UtMetadata, Some(5))
    }

    fn on_update(&mut self, info: &PeerInfo, extended: &ExtendedPeerInfo) {
        self.add_peer(*info, extended).expect(
            "bittorrent-protocol_select: UtMetadataModule::on_update Failed To Add Peer...",
        );
    }
}

//-------------------------------------------------------------------------------//

impl Run for UtMetadataModule {

    fn send(&mut self, item: IDiscoveryMessage) -> Result<Option<IDiscoveryMessage>, DiscoveryError> {
        let start_send = match item {
            IDiscoveryMessage::Control(ControlMessage::AddTorrent(metainfo)) => {
                self.add_torrent(metainfo)
            }
            IDiscoveryMessage::Control(ControlMessage::RemoveTorrent(metainfo)) => {
                self.remove_torrent(metainfo)
            }
            // Dont add the peer yet, use listener to get notified when they send extension messages
            IDiscoveryMessage::Control(ControlMessage::PeerConnected(_)) => Ok(None),
            IDiscoveryMessage::Control(ControlMessage::PeerDisconnected(info)) => {
                self.remove_peer(info)
            }
            IDiscoveryMessage::Control(ControlMessage::Tick(duration)) => self.apply_tick(duration),
            IDiscoveryMessage::DownloadMetainfo(hash) => self.download_metainfo(hash),
            IDiscoveryMessage::ReceivedUtMetadataMessage(info, UtMetadataMessage::Request(msg)) => {
                self.recv_request(info, msg)
            }
            IDiscoveryMessage::ReceivedUtMetadataMessage(info, UtMetadataMessage::Data(msg)) => {
                self.recv_data(info, msg)
            }
            IDiscoveryMessage::ReceivedUtMetadataMessage(info, UtMetadataMessage::Reject(msg)) => {
                self.recv_reject(info, msg)
            }
        };

        // Check if we need to unblock the stream after performing our work
        self.check_stream_unblock();

        start_send
    }

    fn poll(&mut self) -> Option<Result<ODiscoveryMessage, DiscoveryError>> {
        // Check if we completed any downloads
        // Or if we can send any requests
        // Or if we can send any responses
        let opt_result = self
            .retrieve_completed_download()
            .or_else(|| self.retrieve_piece_request())
            .or_else(|| self.retrieve_piece_response());

        // Check if we can unblock the sink after performing our work
        self.check_sink_unblock();

        opt_result
    }
}
