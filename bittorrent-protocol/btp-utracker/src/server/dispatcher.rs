use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use std::thread::{self};

use crate::announce::{AnnounceEvent, AnnounceRequest, AnnounceResponse};
use crate::error::ErrorResponse;
use crate::request::{RequestType, TrackerRequest};
use crate::response::{ResponseType, TrackerResponse};
use crate::scrape::{ScrapeRequest, ScrapeResponse, ScrapeStats};
use nom::IResult;
use tokio::net::UdpSocket;
use tokio::{select, task};
use tokio::sync::mpsc;
use btp_util::bt::InfoHash;
use btp_util::trans::old::TIDGenerator;
use crate::contact::{CompactPeers, CompactPeersV4, CompactPeersV6};
// use umio::external::Sender;
// use umio::{Dispatcher, ELoopBuilder, Provider};
use crate::socket::UtSocket;

const EXPECTED_PACKET_LENGTH: usize = 1500;
const NUM_PEERS_RETURNED: usize = 20;

/// Internal dispatch message for servers.
#[derive(Debug)]
pub enum DispatchMessage {
    Shutdown,
}

/// Create a new background dispatcher to service requests.
pub async fn create_dispatcher(bind: SocketAddr) -> io::Result<mpsc::UnboundedSender<DispatchMessage>>
{

    let udp_socket = UdpSocket::bind(bind).await?;
    let socket_arc = Arc::new(UtSocket::new(udp_socket).unwrap());

    // 专门用一个协程发送数据，避免 数据处理中心 阻塞。
    let send_socket =socket_arc.clone();
    let (message_tx, mut message_rx)=mpsc::unbounded_channel::<(Vec<u8>, SocketAddr)>();
    //专门用一个协程发送数据，避免 数据处理中心 阻塞。
    task::spawn(async move {

        log::info!("消息发送协程已启动");
        while let Some((buffer,addr)) = message_rx.recv().await{
            send_socket.send(&buffer,addr).await;
        }
        log::warn!("消息发送协程已退出");

    });

    let (command_tx, command_rx) = mpsc::unbounded_channel::<DispatchMessage>();
    let mut dispatch = ServerDispatcher::new(command_rx,socket_arc,message_tx);

    task::spawn(dispatch.run_task());

    Ok(command_tx)
}

//----------------------------------------------------------------------------//

/// Dispatcher that executes requests asynchronously.
struct ServerDispatcher {
    is_run: bool,
    command_in: mpsc::UnboundedReceiver<DispatchMessage>,
    incoming: Arc<UtSocket>,
    message_send:  mpsc::UnboundedSender<(Vec<u8>, SocketAddr)>,
    cids: HashSet<u64>,
    cid_generator: TIDGenerator<u64>,
    peers_map: HashMap<InfoHash, HashSet<SocketAddr>>,
}

impl ServerDispatcher
{
    /// Create a new ServerDispatcher.
    fn new(
           cmd_rx: mpsc::UnboundedReceiver<DispatchMessage>,
           incom: Arc<UtSocket>,
           msg_send: mpsc::UnboundedSender<(Vec<u8>,SocketAddr)>) -> ServerDispatcher {
        ServerDispatcher {
            is_run: true,
            incoming:incom,
            message_send: msg_send,
            command_in: cmd_rx,
            cids: HashSet::new(),
            cid_generator: TIDGenerator::<u64>::new(),
            peers_map: HashMap::new(),
        }
    }


    pub(crate) async fn run_task(mut self){
        while  self.is_run {
            self.run_one().await;
        }

    }

    async fn run_one(&mut self){
        select! {

            msg  = self.incoming.recv() => {
                if let Ok((message,addr)) = msg {
                    self.incoming_in_handler(&message[..],addr).await
                } else {
                    self.shutdown()
                }
           }

           cmd  = self.command_in.recv() => {
                if let Some(command) = cmd {
                    self.command_in_handler(command).await
                } else {
                    self.shutdown()
                }
           }
        }
    }

    async fn incoming_in_handler(&mut self, message : &[u8], addr: SocketAddr){
        let request = match TrackerRequest::from_bytes(message) {
            IResult::Done(_, req) => req,
            _ => return, // TODO: Add Logging
        };

        self.process_request(request, addr);
    }

    async fn command_in_handler(&mut self, message : DispatchMessage){
        match message {
          DispatchMessage::Shutdown => self.shutdown(),
        }
    }


    fn shutdown(&mut self){
        self.is_run = false;
    }
    /// Forward the request on to the appropriate handler method.
    fn process_request(
        &mut self,
        request: TrackerRequest,
        addr: SocketAddr,
    ) {
        let conn_id = request.connection_id();
        let trans_id = request.transaction_id();

        match request.request_type() {
            &RequestType::Connect => {
                if conn_id == crate::request::CONNECT_ID_PROTOCOL_ID {
                    self.forward_connect(trans_id, addr);
                } // TODO: Add Logging
            }
            &RequestType::Announce(ref req) => {
                self.forward_announce(trans_id, conn_id, req, addr);
            }
            &RequestType::Scrape(ref req) => {
                self.forward_scrape(trans_id, conn_id, req, addr);
            }
        };
    }

    /// Forward a connect request on to the appropriate handler method.
    fn forward_connect(
        &mut self,
        trans_id: u32,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();

        let cid = self.cid_generator.generate();
        self.cids.insert(cid);
        let response_type = ResponseType::Connect(cid);
        let response = TrackerResponse::new(trans_id, response_type);
        write_response(response, addr,msg_send);
    }

    /// Forward an announce request on to the appropriate handler method.
    fn forward_announce<'b>(
        &mut self,
        // provider: &mut Provider<'a, ServerDispatcher<H>>,
        trans_id: u32,
        conn_id: u64,
        request: &AnnounceRequest<'b>,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();

        if self.cids.contains(&conn_id) {
            let peers = self
                .peers_map
                .entry(request.info_hash())
                .or_insert(HashSet::new());

            // Ignore any source ip directives in the request
            let store_addr = match addr {
                SocketAddr::V4(v4_addr) => {
                    SocketAddr::V4(SocketAddrV4::new(*v4_addr.ip(), request.port()))
                }
                SocketAddr::V6(v6_addr) => {
                    SocketAddr::V6(SocketAddrV6::new(*v6_addr.ip(), request.port(), 0, 0))
                }
            };

            // Resolve what to do with the event
            match request.state().event() {
                AnnounceEvent::None => peers.insert(store_addr),
                AnnounceEvent::Completed => peers.insert(store_addr),
                AnnounceEvent::Started => peers.insert(store_addr),
                AnnounceEvent::Stopped => peers.remove(&store_addr),
            };

            // Check what type of peers the request warrants
            let compact_peers = if request.source_ip().is_ipv4() {
                let mut v4_peers = CompactPeersV4::new();

                for v4_addr in peers
                    .iter()
                    .filter_map(|addr|
                        match addr {
                        &SocketAddr::V4(v4_addr) => Some(v4_addr),
                        &SocketAddr::V6(_) => None,
                    })
                    .take(NUM_PEERS_RETURNED)
                {
                    v4_peers.insert(v4_addr);
                }

                CompactPeers::V4(v4_peers)
            } else {
                let mut v6_peers = CompactPeersV6::new();

                for v6_addr in peers
                    .iter()
                    .filter_map(|addr| match addr {
                        &SocketAddr::V4(_) => None,
                        &SocketAddr::V6(v6_addr) => Some(v6_addr),
                    })
                    .take(NUM_PEERS_RETURNED)
                {
                    v6_peers.insert(v6_addr);
                }

                CompactPeers::V6(v6_peers)
            };

            let msg= ResponseType::Announce(AnnounceResponse::new(
                                    1800,
                                    peers.len() as i32,
                                    peers.len() as i32,
                                    compact_peers));

            let response = TrackerResponse::new(trans_id, msg);
            write_response(response, addr,msg_send);

        } else {

            let msg=  ResponseType::Error(ErrorResponse::new("Connection ID Is Invalid"));
            let response = TrackerResponse::new(trans_id, msg);
            write_response(response, addr, msg_send);

        }
    }

    /// Forward a scrape request on to the appropriate handler method.
    fn forward_scrape<'b>(
        &mut self,
        trans_id: u32,
        conn_id: u64,
        request: &ScrapeRequest<'b>,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();

        if self.cids.contains(&conn_id) {
            let mut response = ScrapeResponse::new();

            for hash in request.iter() {
                let peers = self.peers_map.entry(hash).or_insert(HashSet::new());

                response.insert(ScrapeStats::new(peers.len() as i32, 0, peers.len() as i32));
            }


            let msg = ResponseType::Scrape(response);
            let response = TrackerResponse::new(trans_id, msg);
            write_response(response, addr,msg_send);

        } else {

            let msg = ResponseType::Error(ErrorResponse::new("Connection ID Is Invalid"));
            let response = TrackerResponse::new(trans_id, msg);
            write_response(response, addr,msg_send);
        }
    }

}

/// Write the given tracker response through to the given provider.
fn write_response(
    response: TrackerResponse,
    addr: SocketAddr,
    msg_send: mpsc::UnboundedSender<(Vec<u8>, SocketAddr)>,
) {
    let mut buffer= vec![0_u8;1500];
    let mut writer = Cursor::new(&mut buffer);
    let write_success = response.write_bytes(&mut writer).is_ok();

    if write_success {
        let posi = writer.position() as usize;
        msg_send
            .send((buffer[0..posi].to_vec(), addr))
            .expect("msg send fail");
    }

}

